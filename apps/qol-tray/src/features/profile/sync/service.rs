use anyhow::{Context, Result};
use qol_profile_sync::{
    backup_file_path, build_status, ensure_sync_dirs, filename_string, list_backup_entries,
    load_state_file, load_toggles, merge_profile_with, mergeable_path, now_rfc3339,
    promote::{promote_clone_git_dir, promotion_scope, promotion_scope_label, PromotionScope},
    reconcile, repair_profile_schema, save_state_file, save_toggles, ConflictChoice, FieldConflict,
    GitRepo, ProfileSnapshot, PullOutcome, ResolvableConflict, Side, SignatureSpec,
    SyncActionResult, SyncBackupEntry, SyncBackupPreview, SyncConnectRequest, SyncIncident,
    SyncIncidentKind, SyncLock, SyncPaths, SyncStateFile, SyncStatus, SyncToggles,
};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;
use tokio::sync::Mutex as AsyncMutex;

use crate::features::profile::registry::{
    clear_sync_target, load_sync_target, save_sync_target, SyncTarget,
};
use serde_json::Value;
use std::collections::BTreeMap;

const AUTO_PUSH_INTERVAL_SECS: u64 = 10;
const DEFAULT_REPO_NAME: &str = "qol-tray-profiles";

fn sync_paths() -> Result<SyncPaths> {
    Ok(SyncPaths::new(crate::paths::profile_dir()?))
}

pub struct SyncService {
    state: Mutex<SyncStateFile>,
    toggles: Mutex<SyncToggles>,
    operation_lock: AsyncMutex<()>,
    http: reqwest::Client,
}

impl SyncService {
    pub fn new(_plugins_dir: PathBuf) -> Result<Self> {
        ensure_sync_dirs(&sync_paths()?).ok();
        let state = sync_paths()
            .ok()
            .and_then(|paths| load_state_file(&paths).ok())
            .unwrap_or_default();
        let toggles = sync_paths()
            .ok()
            .and_then(|paths| load_toggles(&paths).ok())
            .unwrap_or_default();
        Ok(Self {
            state: Mutex::new(state),
            toggles: Mutex::new(toggles),
            operation_lock: AsyncMutex::new(()),
            http: reqwest::Client::new(),
        })
    }

    pub fn auto_push_interval() -> Duration {
        Duration::from_secs(AUTO_PUSH_INTERVAL_SECS)
    }

    pub fn status(&self) -> SyncStatus {
        let state = self.snapshot_state();
        let target = load_sync_target().ok().flatten();
        self.build_status_with(&state, target.as_ref())
    }

    pub async fn connect(&self, request: SyncConnectRequest) -> Result<SyncActionResult> {
        let _operation = self.operation_lock.lock().await;
        let token = require_github_token()?;
        let target = match resolve_target(&request, &self.http, &token).await {
            Ok(target) => target,
            Err(error) => return self.return_unpersisted_error(error),
        };
        let repo_path = crate::paths::profile_dir()?;
        std::fs::create_dir_all(&repo_path)?;
        let lock_path = SyncPaths::new(repo_path.clone()).lock_path();
        let has_content = existing_remote_has_content(&self.http, &target, &token).await?;
        let clone_target = target.clone();
        let repo = tokio::task::spawn_blocking(move || -> Result<GitRepo> {
            let _sync_lock = SyncLock::acquire(&lock_path)?;
            let repo = if has_content {
                clone_remote_via_staging(&clone_target, &repo_path, &token)?
            } else {
                let repo = GitRepo::init(&repo_path, &clone_target.repo_url)?;
                ensure_gitignore(&repo_path)?;
                repo.commit_all(
                    "qol-tray: initial commit",
                    &SignatureSpec::default_for_app(),
                )?;
                repo.push(Some(&token))?;
                repo
            };
            commit_profile_schema_repair(&repo_path, &repo, Some(&token))?;
            Ok(repo)
        })
        .await
        .context("join sync connect task")??;

        save_sync_target(&target)?;
        let new_toggles = SyncToggles {
            pull_on_launch: request.pull_on_launch,
            push_on_change: request.push_on_change,
        };
        save_toggles(&sync_paths()?, new_toggles)?;
        *self.toggles_mut() = new_toggles;

        let head = repo.head_sha()?;
        let mut state = self.state_mut();
        state.head_sha = head;
        state.last_sync_at = Some(now_rfc3339());
        state.last_error = None;
        state.incident = None;
        save_state_file(&sync_paths()?, &state)?;
        let saved = state.clone();
        drop(state);

        Ok(SyncActionResult {
            message: "Cloud sync connected".to_string(),
            applied_remote: true,
            status: self.build_status_with(&saved, Some(&target)),
        })
    }

    pub async fn bootstrap_github_connect(&self) -> Result<SyncActionResult> {
        self.connect(SyncConnectRequest {
            repo_url: None,
            auto_create: true,
            pull_on_launch: true,
            push_on_change: true,
        })
        .await
    }

    pub async fn disconnect(&self) -> Result<SyncActionResult> {
        let _operation = self.operation_lock.lock().await;
        clear_sync_target()?;
        let repo_path = crate::paths::profile_dir()?;
        let lock_path = SyncPaths::new(repo_path.clone()).lock_path();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let _sync_lock = SyncLock::acquire(&lock_path)?;
            let git_dir = repo_path.join(".git");
            if git_dir.exists() {
                std::fs::remove_dir_all(&git_dir).ok();
            }
            Ok(())
        })
        .await
        .context("join sync disconnect task")??;
        let mut state = self.state_mut();
        *state = SyncStateFile::default();
        save_state_file(&sync_paths()?, &state)?;
        let cleared = state.clone();
        drop(state);
        Ok(SyncActionResult {
            message: "Cloud sync disconnected".to_string(),
            applied_remote: false,
            status: self.build_status_with(&cleared, None),
        })
    }

    pub async fn manual_pull(&self) -> Result<SyncActionResult> {
        self.do_pull().await
    }

    pub async fn pull_on_launch(&self) -> Result<SyncActionResult> {
        if !self.snapshot_toggles().pull_on_launch {
            return self.noop_result("Pull on launch disabled");
        }
        self.do_pull().await
    }

    pub async fn manual_push(&self) -> Result<SyncActionResult> {
        self.do_push("manual push").await
    }

    pub async fn auto_push_if_dirty(&self) -> Result<SyncActionResult> {
        if !self.snapshot_toggles().push_on_change {
            return self.noop_result("Auto-push disabled");
        }
        if self.snapshot_state().incident.is_some() {
            return self.noop_result("Skipped auto-push (incident pending)");
        }
        if !self.needs_push().await? {
            return self.noop_result("Nothing to push");
        }
        self.do_push("auto push").await
    }

    async fn needs_push(&self) -> Result<bool> {
        if load_sync_target()?.is_none() {
            return Ok(false);
        }
        let repo_path = crate::paths::profile_dir()?;
        let last_pushed = self.snapshot_state().head_sha;
        tokio::task::spawn_blocking(move || -> Result<bool> {
            let Ok(repo) = GitRepo::open(&repo_path) else {
                return Ok(false);
            };
            if repo.is_dirty()? {
                return Ok(true);
            }
            Ok(repo.head_sha()? != last_pushed)
        })
        .await
        .context("join sync dirty-check")?
    }

    pub fn list_conflicts(&self) -> Vec<ResolvableConflict> {
        self.snapshot_state().conflicts
    }

    pub fn open_backups_dir(&self) -> Result<()> {
        ensure_sync_dirs(&sync_paths()?)?;
        let dir = sync_paths()?.backups_dir();
        super::open_dir(&dir)
    }

    pub fn open_backup_file(&self, file_name: &str) -> Result<()> {
        let path = backup_file_path(&sync_paths()?, file_name)?;
        trace_backup_file("PROFILE_BACKUP_OPEN", file_name, &path);
        if !path.exists() {
            anyhow::bail!("backup not found");
        }
        super::open_path(&path)
    }

    pub fn list_backups(&self) -> Result<Vec<SyncBackupEntry>> {
        let dir = sync_paths().ok().map(|paths| paths.backups_dir());
        Ok(list_backup_entries(dir.as_deref()))
    }

    pub fn preview_backup(&self, file_name: &str) -> Result<SyncBackupPreview> {
        let path = backup_file_path(&sync_paths()?, file_name)?;
        trace_backup_file("PROFILE_BACKUP_PREVIEW", file_name, &path);
        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("read backup {}", path.display()))?;
        Ok(SyncBackupPreview {
            file_name: filename_string(&path),
            content,
        })
    }

    async fn do_pull(&self) -> Result<SyncActionResult> {
        let _operation = self.operation_lock.lock().await;
        let Some(target) = load_sync_target()? else {
            return self.noop_result("Sync not configured");
        };
        let token = require_github_token()?;
        let repo_path = crate::paths::profile_dir()?;
        let lock_path = SyncPaths::new(repo_path.clone()).lock_path();
        let clone_target = target.clone();
        let pulled = tokio::task::spawn_blocking(move || -> Result<PullTaskOutput> {
            let _sync_lock = SyncLock::acquire(&lock_path)?;
            let (repo, cloned) = match GitRepo::open(&repo_path) {
                Ok(repo) => (repo, false),
                Err(_) => (
                    clone_remote_via_staging(&clone_target, &repo_path, &token)?,
                    true,
                ),
            };
            let outcome = repo.fetch(Some(&token))?;
            let mut applied_remote = cloned;
            if matches!(outcome, PullOutcome::FastForwarded { .. }) {
                apply_fast_forward(&repo, &outcome, "pull")?;
                applied_remote = true;
            }
            let mut conflicts = Vec::new();
            let mut auto_applied = false;
            if matches!(outcome, PullOutcome::Diverged { .. }) {
                let merge = reconcile(&repo)?;
                if merge.conflicts.is_empty() {
                    apply_merged_profile(&repo, &repo_path, &merge.merged, "pull")?;
                    repo.commit_all("merge remote changes", &SignatureSpec::default_for_app())?;
                    repo.push(Some(&token))?;
                    auto_applied = true;
                    applied_remote = true;
                } else {
                    conflicts = decorate_conflicts(&repo, merge.conflicts)?;
                }
            }
            if !matches!(outcome, PullOutcome::Diverged { .. }) {
                commit_profile_schema_repair(&repo_path, &repo, Some(&token))?;
            }
            let head = repo.head_sha()?;
            Ok(PullTaskOutput {
                outcome,
                head,
                conflicts,
                auto_applied,
                applied_remote,
            })
        })
        .await
        .context("join sync pull task")?;
        let output = match pulled {
            Ok(value) => value,
            Err(error) => return self.persisted_error(error, Some(&target)),
        };
        let PullTaskOutput {
            outcome,
            head,
            conflicts,
            auto_applied,
            applied_remote,
        } = output;
        let message = match &outcome {
            _ if applied_remote && matches!(outcome, PullOutcome::AlreadyUpToDate) => {
                "Pulled changes from remote".to_string()
            }
            PullOutcome::AlreadyUpToDate => "Already up to date".to_string(),
            PullOutcome::FastForwarded { .. } => "Pulled changes from remote".to_string(),
            PullOutcome::Diverged { .. } if auto_applied => "Merged remote changes".to_string(),
            PullOutcome::Diverged { .. } => format!("{} setting(s) need review", conflicts.len()),
        };
        let mut state = self.state_mut();
        if conflicts.is_empty() {
            state.incident = None;
            state.last_error = None;
            state.conflicts.clear();
        } else {
            let (local, remote) = match &outcome {
                PullOutcome::Diverged { local, remote } => (local.clone(), remote.clone()),
                _ => (String::new(), String::new()),
            };
            state.incident = Some(SyncIncident {
                kind: SyncIncidentKind::Conflict,
                message: format!(
                    "{} setting(s) differ (local {} vs remote {})",
                    conflicts.len(),
                    short_sha(&local),
                    short_sha(&remote)
                ),
                backup_file: None,
                created_at: now_rfc3339(),
            });
            state.conflicts = conflicts;
        }
        state.head_sha = head;
        state.last_sync_at = Some(now_rfc3339());
        save_state_file(&sync_paths()?, &state)?;
        let saved = state.clone();
        drop(state);
        Ok(SyncActionResult {
            message,
            applied_remote,
            status: self.build_status_with(&saved, Some(&target)),
        })
    }

    pub async fn resolve_conflicts(
        &self,
        choices: Vec<ConflictChoice>,
    ) -> Result<SyncActionResult> {
        let _operation = self.operation_lock.lock().await;
        let Some(target) = load_sync_target()? else {
            return self.noop_result("Sync not configured");
        };
        let token = require_github_token()?;
        let repo_path = crate::paths::profile_dir()?;
        let lock_path = SyncPaths::new(repo_path.clone()).lock_path();
        let resolved = tokio::task::spawn_blocking(move || -> Result<Option<String>> {
            let _sync_lock = SyncLock::acquire(&lock_path)?;
            let repo = GitRepo::open(&repo_path)?;
            let local_oid = repo
                .local_oid()?
                .ok_or_else(|| anyhow::anyhow!("local branch has no commit"))?;
            let remote_oid = repo.remote_oid()?;
            let base: BTreeMap<String, Value> = match repo.merge_base_with_remote()? {
                Some(oid) => repo.snapshot_json_at(oid, mergeable_path)?,
                None => BTreeMap::new(),
            };
            let local = repo.snapshot_json_at(local_oid, mergeable_path)?;
            let remote = repo.snapshot_json_at(remote_oid, mergeable_path)?;
            write_conflict_backup(&serde_json::json!({ "local": local, "remote": remote }))?;
            let resolve = |file: &str, key: &str| -> Option<bool> {
                choices
                    .iter()
                    .find(|choice| choice.file == file && choice.key_path == key)
                    .map(|choice| matches!(choice.side, Side::Remote))
            };
            let merged = merge_profile_with(
                &ProfileSnapshot { files: base },
                &ProfileSnapshot { files: local },
                &ProfileSnapshot { files: remote },
                &resolve,
            );
            apply_merged_profile(&repo, &repo_path, &merged.merged, "resolve_conflicts")?;
            repo.commit_all("resolve sync conflicts", &SignatureSpec::default_for_app())?;
            repo.push(Some(&token))?;
            repo.head_sha()
        })
        .await
        .context("join resolve task")?;
        let head = match resolved {
            Ok(value) => value,
            Err(error) => return self.persisted_error(error, Some(&target)),
        };
        let mut state = self.state_mut();
        state.conflicts.clear();
        state.incident = None;
        state.last_error = None;
        state.head_sha = head;
        state.last_sync_at = Some(now_rfc3339());
        save_state_file(&sync_paths()?, &state)?;
        let saved = state.clone();
        drop(state);
        Ok(SyncActionResult {
            message: "Conflicts resolved".to_string(),
            applied_remote: true,
            status: self.build_status_with(&saved, Some(&target)),
        })
    }

    async fn do_push(&self, reason: &str) -> Result<SyncActionResult> {
        let _operation = self.operation_lock.lock().await;
        let Some(target) = load_sync_target()? else {
            return self.noop_result("Sync not configured");
        };
        let token = require_github_token()?;
        let repo_path = crate::paths::profile_dir()?;
        let lock_path = SyncPaths::new(repo_path.clone()).lock_path();
        let reason_owned = reason.to_string();
        let pushed = tokio::task::spawn_blocking(move || -> Result<PushTaskOutput> {
            let _sync_lock = SyncLock::acquire(&lock_path)?;
            push_profile_changes(&repo_path, Some(&token), &reason_owned)
        })
        .await
        .context("join sync push task")?;
        let output = match pushed {
            Ok(value) => value,
            Err(error) => {
                trace_profile_sync_push("rejected", false, false, None);
                return self.persisted_error(error, Some(&target));
            }
        };
        let message = if !output.conflicts.is_empty() {
            format!("{} setting(s) need review", output.conflicts.len())
        } else if output.pushed {
            "Pushed changes to remote".to_string()
        } else if output.applied_remote {
            "Pulled changes from remote".to_string()
        } else {
            "Nothing to push".to_string()
        };
        let mut state = self.state_mut();
        if output.conflicts.is_empty() {
            state.conflicts.clear();
            state.incident = None;
            state.last_error = None;
        } else {
            state.incident = Some(SyncIncident {
                kind: SyncIncidentKind::Conflict,
                message: format!("{} setting(s) need review", output.conflicts.len()),
                backup_file: None,
                created_at: now_rfc3339(),
            });
            state.conflicts = output.conflicts;
            state.last_error = None;
        }
        let head = output.head.clone();
        state.head_sha = output.head;
        state.last_sync_at = Some(now_rfc3339());
        save_state_file(&sync_paths()?, &state)?;
        trace_profile_sync_push("accepted", output.pushed, true, head.as_deref());
        let saved = state.clone();
        drop(state);
        Ok(SyncActionResult {
            message,
            applied_remote: output.applied_remote,
            status: self.build_status_with(&saved, Some(&target)),
        })
    }

    fn noop_result(&self, message: &str) -> Result<SyncActionResult> {
        let state = self.snapshot_state();
        let target = load_sync_target().ok().flatten();
        Ok(SyncActionResult {
            message: message.to_string(),
            applied_remote: false,
            status: self.build_status_with(&state, target.as_ref()),
        })
    }

    fn return_unpersisted_error(&self, error: anyhow::Error) -> Result<SyncActionResult> {
        let state = self.snapshot_state();
        let target = load_sync_target().ok().flatten();
        Ok(SyncActionResult {
            message: format!("Sync failed: {error:#}"),
            applied_remote: false,
            status: self.build_status_with(&state, target.as_ref()),
        })
    }

    fn persisted_error(
        &self,
        error: anyhow::Error,
        target: Option<&SyncTarget>,
    ) -> Result<SyncActionResult> {
        let mut state = self.state_mut();
        state.last_error = Some(format!("{error:#}"));
        save_state_file(&sync_paths()?, &state)?;
        let saved = state.clone();
        drop(state);
        Ok(SyncActionResult {
            message: format!("Sync failed: {error:#}"),
            applied_remote: false,
            status: self.build_status_with(&saved, target),
        })
    }

    fn build_status_with(&self, state: &SyncStateFile, target: Option<&SyncTarget>) -> SyncStatus {
        let toggles = self.snapshot_toggles();
        let backups_dir = sync_paths().ok().map(|paths| paths.backups_dir());
        let backup_files = list_backup_entries(backups_dir.as_deref());
        build_status(
            state,
            target,
            toggles,
            crate::credentials::github_bearer_token().is_some(),
            backups_dir.as_deref(),
            &backup_files,
        )
    }

    fn state_mut(&self) -> std::sync::MutexGuard<'_, SyncStateFile> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn snapshot_state(&self) -> SyncStateFile {
        self.state_mut().clone()
    }

    fn toggles_mut(&self) -> std::sync::MutexGuard<'_, SyncToggles> {
        self.toggles.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn snapshot_toggles(&self) -> SyncToggles {
        *self.toggles_mut()
    }
}

fn trace_backup_file(tag: &str, file_name: &str, path: &Path) {
    #[cfg(debug_assertions)]
    {
        qol_runtime::probe!(
            tag,
            "file={:?} path_kind={}",
            file_name,
            trace_path_kind(path)
        );
    }
    #[cfg(not(debug_assertions))]
    let _ = (tag, file_name, path);
}

#[cfg(debug_assertions)]
fn trace_path_kind(path: &Path) -> &'static str {
    let Ok(metadata) = std::fs::symlink_metadata(path) else {
        return "missing";
    };
    if metadata.file_type().is_symlink() {
        return "symlink";
    }
    if metadata.is_dir() {
        return "dir";
    }
    if metadata.is_file() {
        return "file";
    }
    "other"
}

fn require_github_token() -> Result<String> {
    let token = crate::features::github_auth::oauth_access_token()
        .or_else(crate::credentials::github_bearer_token)
        .context("GitHub account is not connected")?;
    crate::features::auth::ensure_scope(crate::features::auth::Scope::GitHub(
        crate::features::auth::GitHubScope::Repo,
    ))?;
    Ok(token)
}

async fn resolve_target(
    request: &SyncConnectRequest,
    http: &reqwest::Client,
    token: &str,
) -> Result<SyncTarget> {
    if let Some(url) = request
        .repo_url
        .as_ref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
    {
        return Ok(SyncTarget {
            repo_url: url,
            auto_created: false,
        });
    }
    if !request.auto_create {
        anyhow::bail!("repo_url is required when auto_create is false");
    }
    let url = ensure_auto_created_repo(http, token).await?;
    Ok(SyncTarget {
        repo_url: url,
        auto_created: true,
    })
}

async fn ensure_auto_created_repo(http: &reqwest::Client, token: &str) -> Result<String> {
    let login = fetch_github_login(http, token).await?;
    let url = format!("https://github.com/{login}/{DEFAULT_REPO_NAME}.git");
    if github_repo_exists(http, token, &login, DEFAULT_REPO_NAME).await? {
        return Ok(url);
    }
    create_github_private_repo(http, token, DEFAULT_REPO_NAME).await?;
    Ok(url)
}

async fn fetch_github_login(http: &reqwest::Client, token: &str) -> Result<String> {
    #[derive(serde::Deserialize)]
    struct User {
        login: String,
    }
    let response = http
        .get("https://api.github.com/user")
        .bearer_auth(token)
        .header("User-Agent", "qol-tray")
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .context("fetch GitHub user")?;
    if !response.status().is_success() {
        anyhow::bail!("GitHub user lookup failed: {}", response.status());
    }
    let user: User = response.json().await.context("decode GitHub user")?;
    Ok(user.login)
}

async fn github_repo_exists(
    http: &reqwest::Client,
    token: &str,
    owner: &str,
    name: &str,
) -> Result<bool> {
    let url = format!("https://api.github.com/repos/{owner}/{name}");
    let response = http
        .get(&url)
        .bearer_auth(token)
        .header("User-Agent", "qol-tray")
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .context("check GitHub repo existence")?;
    Ok(response.status().is_success())
}

async fn create_github_private_repo(http: &reqwest::Client, token: &str, name: &str) -> Result<()> {
    let body = serde_json::json!({
        "name": name,
        "private": true,
        "auto_init": false,
        "description": "qol-tray profile sync",
    });
    let response = http
        .post("https://api.github.com/user/repos")
        .bearer_auth(token)
        .header("User-Agent", "qol-tray")
        .header("Accept", "application/vnd.github+json")
        .json(&body)
        .send()
        .await
        .context("create GitHub repo")?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!("create repo {name} failed: {status}: {body}");
    }
    Ok(())
}

async fn existing_remote_has_content(
    http: &reqwest::Client,
    target: &SyncTarget,
    token: &str,
) -> Result<bool> {
    if !target.auto_created {
        return Ok(true);
    }
    let Some((owner, name)) = parse_owner_name(&target.repo_url) else {
        return Ok(true);
    };
    let url = format!("https://api.github.com/repos/{owner}/{name}/branches");
    let response = http
        .get(&url)
        .bearer_auth(token)
        .header("User-Agent", "qol-tray")
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .context("list GitHub branches")?;
    if !response.status().is_success() {
        return Ok(false);
    }
    let branches: Vec<serde_json::Value> = response.json().await.unwrap_or_default();
    Ok(!branches.is_empty())
}

fn parse_owner_name(url: &str) -> Option<(String, String)> {
    let stripped = url.trim_end_matches(".git");
    let rest = stripped.split("github.com/").nth(1)?;
    let mut parts = rest.split('/');
    let owner = parts.next()?.to_string();
    let name = parts.next()?.to_string();
    if owner.is_empty() || name.is_empty() {
        return None;
    }
    Some((owner, name))
}

fn clone_remote_via_staging(
    target: &SyncTarget,
    profile_dir: &Path,
    token: &str,
) -> Result<GitRepo> {
    let staging = create_staging_dir(profile_dir)?;
    let outcome = clone_into_staging_then_promote(target, &staging, profile_dir, token);
    if let Err(error) = std::fs::remove_dir_all(&staging) {
        log::warn!(
            "[sync] cleanup of staging dir {} failed: {error:#}",
            staging.display()
        );
    }
    outcome
}

fn clone_into_staging_then_promote(
    target: &SyncTarget,
    staging: &Path,
    profile_dir: &Path,
    token: &str,
) -> Result<GitRepo> {
    GitRepo::clone(&target.repo_url, staging, Some(token))?;
    promote_allowlisted_clone(staging, profile_dir)?;
    promote_clone_git_dir(staging, profile_dir)?;
    GitRepo::open(profile_dir).context("open promoted profile repo")
}

/// Promotes an allowlisted clone while holding the runtime-config guards:
/// classifies the touched plugin scope from the staging tree, acquires the
/// matching profile-config write guard, then delegates the copy to the
/// shared engine. The generation-bearing trace summary stays tray-side.
pub(crate) fn promote_allowlisted_clone(staging: &Path, profile: &Path) -> Result<()> {
    let _mutation = crate::plugins::config::begin_runtime_config_global_mutation();
    let scope = promotion_scope(staging)?;
    let scope_label = promotion_scope_label(&scope);
    let _profile_guard = match scope {
        PromotionScope::All => crate::plugins::config::profile_config_write_guard(),
        PromotionScope::Plugins(plugin_ids) => {
            crate::plugins::config::profile_config_write_guard_for_plugins(plugin_ids)
        }
    };
    let generation = crate::plugins::config::current_profile_config_generation();
    qol_profile_sync::promote_allowlisted_clone(staging, profile)?;
    trace_profile_sync_apply("promote", &scope_label, generation);
    Ok(())
}

fn trace_profile_sync_apply(operation: &str, outcome: &str, generation: u64) {
    #[cfg(debug_assertions)]
    qol_runtime::probe!(
        "PROFILE_SYNC_APPLY",
        "operation={operation} outcome={outcome} scope=all profile_generation={generation} consumed_generation={generation} acknowledged_generation=none"
    );
    #[cfg(not(debug_assertions))]
    let _ = (operation, outcome, generation);
}

fn trace_profile_sync_push(
    outcome: &str,
    transferred: bool,
    head_persisted: bool,
    head: Option<&str>,
) {
    #[cfg(debug_assertions)]
    qol_runtime::probe!(
        "PROFILE_SYNC_PUSH",
        "outcome={outcome} transferred={transferred} head_persisted={head_persisted} head={}",
        head.unwrap_or("-")
    );
    #[cfg(not(debug_assertions))]
    let _ = (outcome, transferred, head_persisted, head);
}

fn create_staging_dir(profile_dir: &Path) -> Result<PathBuf> {
    let parent = profile_dir.parent().ok_or_else(|| {
        anyhow::anyhow!(
            "profile dir {} has no parent for staging",
            profile_dir.display()
        )
    })?;
    let pid = std::process::id();
    for attempt in 0..128 {
        let name = format!(".sync-staging-{pid}-{attempt}");
        let candidate = parent.join(&name);
        if !candidate.exists() {
            std::fs::create_dir_all(&candidate)
                .with_context(|| format!("create staging dir {}", candidate.display()))?;
            return Ok(candidate);
        }
    }
    anyhow::bail!(
        "could not allocate a staging directory under {}",
        parent.display()
    )
}

fn ensure_gitignore(repo_path: &Path) -> Result<()> {
    let path = repo_path.join(".gitignore");
    if path.exists() {
        return Ok(());
    }
    std::fs::write(&path, qol_profile_sync::GITIGNORE_CONTENTS)
        .with_context(|| format!("write {}", path.display()))
}

fn short_sha(sha: &str) -> String {
    sha.chars().take(7).collect()
}

struct PullTaskOutput {
    outcome: PullOutcome,
    head: Option<String>,
    conflicts: Vec<ResolvableConflict>,
    auto_applied: bool,
    applied_remote: bool,
}

struct PushTaskOutput {
    pushed: bool,
    head: Option<String>,
    conflicts: Vec<ResolvableConflict>,
    applied_remote: bool,
}

fn push_profile_changes(
    repo_path: &Path,
    token: Option<&str>,
    reason: &str,
) -> Result<PushTaskOutput> {
    let repo = GitRepo::open(repo_path)?;
    ensure_gitignore(repo_path)?;
    repair_profile_schema_under_guard(repo_path)?;
    repo.commit_all(reason, &SignatureSpec::default_for_app())?;
    let outcome = repo.fetch(token)?;
    let mut conflicts = Vec::new();
    let mut applied_remote = false;

    if matches!(outcome, PullOutcome::FastForwarded { .. }) {
        apply_fast_forward(&repo, &outcome, "push")?;
        applied_remote = true;
    }

    if matches!(outcome, PullOutcome::Diverged { .. }) {
        let merge = reconcile(&repo)?;
        if merge.conflicts.is_empty() {
            apply_merged_profile(&repo, repo_path, &merge.merged, "push")?;
            repo.commit_all("merge remote changes", &SignatureSpec::default_for_app())?;
            applied_remote = true;
        } else {
            conflicts = decorate_conflicts(&repo, merge.conflicts)?;
        }
    }

    if conflicts.is_empty() {
        let pushed = repo.push(token)?.transferred;
        let head = repo.head_sha()?;
        return Ok(PushTaskOutput {
            pushed,
            head,
            conflicts,
            applied_remote,
        });
    }
    let head = repo.head_sha()?;
    Ok(PushTaskOutput {
        pushed: false,
        head,
        conflicts,
        applied_remote,
    })
}

fn write_merged_profile(repo_path: &Path, merged: &BTreeMap<String, Value>) -> Result<()> {
    let _mutation = crate::plugins::config::begin_runtime_config_global_mutation();
    for (rel, value) in merged {
        let path = repo_path.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create dir {}", parent.display()))?;
        }
        let text = serde_json::to_string_pretty(value)?;
        std::fs::write(&path, format!("{text}\n"))
            .with_context(|| format!("write merged {}", path.display()))?;
    }
    repair_profile_schema(repo_path)?;
    Ok(())
}

fn apply_fast_forward(repo: &GitRepo, outcome: &PullOutcome, operation: &str) -> Result<u64> {
    let profile_guard = crate::plugins::config::profile_config_write_guard_unmarked();
    let generation =
        profile_guard.mark_changed(crate::plugins::config::ProfileConfigInvalidation::All);
    repo.apply_pull(outcome)?;
    trace_profile_sync_apply(operation, "fast_forward", generation);
    Ok(generation)
}

fn apply_merged_profile(
    repo: &GitRepo,
    repo_path: &Path,
    merged: &BTreeMap<String, Value>,
    operation: &str,
) -> Result<u64> {
    let profile_guard = crate::plugins::config::profile_config_write_guard_unmarked();
    let generation =
        profile_guard.mark_changed(crate::plugins::config::ProfileConfigInvalidation::All);
    repo.reset_to_remote()?;
    write_merged_profile(repo_path, merged)?;
    trace_profile_sync_apply(operation, "merged", generation);
    Ok(generation)
}

fn commit_profile_schema_repair(
    repo_path: &Path,
    repo: &GitRepo,
    token: Option<&str>,
) -> Result<()> {
    if !repair_profile_schema_under_guard(repo_path)? {
        return Ok(());
    }
    let commit = repo.commit_all("repair profile schema", &SignatureSpec::default_for_app())?;
    if commit.is_some() {
        repo.push(token)?;
    }
    Ok(())
}

fn repair_profile_schema_under_guard(repo_path: &Path) -> Result<bool> {
    let profile_guard = crate::plugins::config::profile_config_write_guard_unmarked();
    let _mutation = crate::plugins::config::begin_runtime_config_global_mutation();
    let changed = repair_profile_schema(repo_path)?;
    if !changed {
        return Ok(false);
    }
    let generation =
        profile_guard.mark_changed(crate::plugins::config::ProfileConfigInvalidation::All);
    trace_profile_sync_apply("schema_repair", "schema", generation);
    Ok(true)
}

fn write_conflict_backup(value: &Value) -> Result<String> {
    qol_profile_sync::write_conflict_backup(&sync_paths()?, value)
}

fn decorate_conflicts(
    repo: &GitRepo,
    conflicts: Vec<FieldConflict>,
) -> Result<Vec<ResolvableConflict>> {
    let local = repo.local_oid()?;
    let remote = repo.remote_oid().ok();
    let mut out = Vec::with_capacity(conflicts.len());
    for conflict in conflicts {
        let key = last_key(&conflict.key_path);
        let local_edited = local.and_then(|oid| {
            repo.field_edited_at(oid, &conflict.file, key)
                .ok()
                .flatten()
        });
        let remote_edited = remote.and_then(|oid| {
            repo.field_edited_at(oid, &conflict.file, key)
                .ok()
                .flatten()
        });
        out.push(ResolvableConflict {
            file: conflict.file,
            plugin: conflict.plugin,
            key_path: conflict.key_path,
            local: conflict.local,
            remote: conflict.remote,
            local_edited,
            remote_edited,
        });
    }
    Ok(out)
}

fn last_key(key_path: &str) -> &str {
    key_path.rsplit('.').next().unwrap_or(key_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use git2::Repository;
    use serde_json::json;
    use std::collections::BTreeMap;
    use std::ffi::OsString;
    use std::path::Path;
    use std::sync::mpsc;
    use std::time::Duration;
    use tempfile::TempDir;

    const ALT_TAB_UID: &str = "a7f48ac7-3cd5-4402-a1fe-d517fbce0fd6";

    fn read_json(path: &std::path::Path) -> Value {
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
    }

    fn init_bare_origin(dir: &Path) -> String {
        Repository::init_bare(dir).unwrap();
        let normalized = dir.display().to_string().replace('\\', "/");
        format!("file:///{}", normalized.trim_start_matches('/'))
    }

    fn write_file(path: &Path, value: &Value) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
    }

    struct TestPathRootEnvGuard {
        previous: Option<OsString>,
    }

    impl TestPathRootEnvGuard {
        fn new(root: &Path) -> Self {
            let previous = std::env::var_os("QOL_TRAY_TEST_PATH_ROOT");
            std::env::set_var("QOL_TRAY_TEST_PATH_ROOT", root);
            Self { previous }
        }
    }

    impl Drop for TestPathRootEnvGuard {
        fn drop(&mut self) {
            if let Some(previous) = &self.previous {
                std::env::set_var("QOL_TRAY_TEST_PATH_ROOT", previous);
                return;
            }
            std::env::remove_var("QOL_TRAY_TEST_PATH_ROOT");
        }
    }

    fn sig() -> SignatureSpec {
        SignatureSpec {
            name: "Tester".to_string(),
            email: "tester@example.com".to_string(),
        }
    }

    fn seed_profile_repo(path: &Path, url: &str) -> GitRepo {
        let repo = GitRepo::init(path, url).unwrap();
        write_file(&path.join("default/manifest.json"), &json!({"version": 1}));
        write_file(
            &path.join("default/core/plugin-configs/plugin-a.json"),
            &json!({"value": 1}),
        );
        repo.commit_all("seed", &sig()).unwrap();
        repo.push(None).unwrap();
        repo
    }

    #[test]
    fn promotion_advances_profile_generation() {
        let tmp = TempDir::new().unwrap();
        let staging = tmp.path().join("staging");
        let profile = tmp.path().join("profile");
        std::fs::create_dir_all(&staging).unwrap();
        std::fs::create_dir_all(&profile).unwrap();
        write_file(
            &staging.join("default/core/plugins.lock.json"),
            &json!({"plugins": []}),
        );
        let before = crate::plugins::config::current_profile_config_generation();

        promote_allowlisted_clone(&staging, &profile).unwrap();

        let after = crate::plugins::config::current_profile_config_generation();
        assert!(after > before);
    }

    #[test]
    fn promotion_waits_for_autostart_profile_read() {
        let tmp = TempDir::new().unwrap();
        let staging = tmp.path().join("staging");
        let profile = tmp.path().join("profile");
        std::fs::create_dir_all(&staging).unwrap();
        std::fs::create_dir_all(&profile).unwrap();
        write_file(
            &staging.join("default/core/plugins.lock.json"),
            &json!({"plugins": []}),
        );
        let read_guard = crate::plugins::config::profile_config_read_guard();
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let staging_for_thread = staging.clone();
        let profile_for_thread = profile.clone();
        let worker = std::thread::spawn(move || {
            started_tx.send(()).unwrap();
            let result = promote_allowlisted_clone(&staging_for_thread, &profile_for_thread);
            done_tx.send(result.is_ok()).unwrap();
        });

        started_rx.recv().unwrap();
        assert!(done_rx.try_recv().is_err());
        drop(read_guard);
        assert!(done_rx.recv().unwrap());
        worker.join().unwrap();
        assert_eq!(
            read_json(&profile.join("default/core/plugins.lock.json")),
            json!({"plugins": []})
        );
    }

    #[test]
    fn write_merged_profile_repairs_legacy_uid_schema() {
        let tmp = TempDir::new().unwrap();
        let repo_path = tmp.path().join("profile");
        let merged = BTreeMap::from([
            ("default/manifest.json".to_string(), json!({"version": 1})),
            (
                "default/core/plugins.lock.json".to_string(),
                json!({
                    "plugins": [{
                        "id": "plugin-alt-tab",
                        "repo_url": "https://example.invalid/plugin-alt-tab.git",
                        "version": "1.0.0",
                        "platforms": ["macos"]
                    }]
                }),
            ),
            (
                "default/os/macos/hotkeys.json".to_string(),
                json!({
                    "hotkeys": [{
                        "id": "hk-alt-tab",
                        "key": "Alt+Tab",
                        "plugin_id": "plugin-alt-tab",
                        "action": "open",
                        "enabled": true
                    }]
                }),
            ),
            (
                "default/core/plugin-configs/plugin-alt-tab.json".to_string(),
                json!({"opacity": 0.8}),
            ),
        ]);

        write_merged_profile(&repo_path, &merged).unwrap();

        let lock = read_json(&repo_path.join("default/core/plugins.lock.json"));
        assert_eq!(lock["plugins"][0]["uid"], ALT_TAB_UID);

        let hotkeys = read_json(&repo_path.join("default/os/macos/hotkeys.json"));
        let binding = hotkeys["hotkeys"][0].as_object().unwrap();
        assert_eq!(binding["plugin_uid"], ALT_TAB_UID);
        assert!(!binding.contains_key("plugin_id"));

        assert!(!repo_path
            .join("default/core/plugin-configs/plugin-alt-tab.json")
            .exists());
        assert!(repo_path
            .join(format!("default/core/plugin-configs/{ALT_TAB_UID}.json"))
            .is_file());
    }

    #[test]
    fn fast_forward_profile_apply_invalidates_generation_for_remote_state() {
        let tmp = TempDir::new().unwrap();
        let url = init_bare_origin(&tmp.path().join("origin.git"));
        let remote_path = tmp.path().join("remote/profile");
        let remote = seed_profile_repo(&remote_path, &url);
        let local_path = tmp.path().join("local/profile");
        let local = GitRepo::clone(&url, &local_path, None).unwrap();

        write_file(
            &remote_path.join("default/core/plugin-configs/plugin-a.json"),
            &json!({"value": 2}),
        );
        write_file(
            &remote_path.join("default/core/plugins.lock.json"),
            &json!({
                "version": 1,
                "plugins": [{"id": "plugin-a", "uid": "remote-uid", "version": "2.0.0"}]
            }),
        );
        remote.commit_all("remote state", &sig()).unwrap();
        remote.push(None).unwrap();

        let outcome = local.fetch(None).unwrap();
        assert!(matches!(outcome, PullOutcome::FastForwarded { .. }));
        let before = crate::plugins::config::current_profile_config_generation();
        apply_fast_forward(&local, &outcome, "test").unwrap();
        let after = crate::plugins::config::current_profile_config_generation();

        assert!(after > before);
        assert_eq!(
            read_json(&local_path.join("default/core/plugin-configs/plugin-a.json")),
            json!({"value": 2})
        );
        assert_eq!(
            read_json(&local_path.join("default/core/plugins.lock.json"))["plugins"][0]["uid"],
            "remote-uid"
        );
    }

    #[test]
    fn fetch_waits_for_scoped_write_before_checkout_apply() {
        let tmp = TempDir::new().unwrap();
        let url = init_bare_origin(&tmp.path().join("origin.git"));
        let remote_path = tmp.path().join("remote/profile");
        let remote = seed_profile_repo(&remote_path, &url);
        let local_path = tmp.path().join("local/profile");
        let local = GitRepo::clone(&url, &local_path, None).unwrap();

        write_file(
            &remote_path.join("default/core/plugin-configs/plugin-a.json"),
            &json!({"value": 2}),
        );
        remote.commit_all("remote state", &sig()).unwrap();
        remote.push(None).unwrap();

        let (writer_ready_tx, writer_ready_rx) = mpsc::channel();
        let (writer_release_tx, writer_release_rx) = mpsc::channel();
        let writer = std::thread::spawn(move || {
            let _guard = crate::plugins::config::profile_config_write_guard_for_plugin("plugin-a");
            writer_ready_tx.send(()).unwrap();
            writer_release_rx.recv().unwrap();
        });
        writer_ready_rx
            .recv_timeout(Duration::from_secs(1))
            .unwrap();

        let outcome = local.fetch(None).unwrap();
        assert!(matches!(outcome, PullOutcome::FastForwarded { .. }));
        assert_eq!(
            read_json(&local_path.join("default/core/plugin-configs/plugin-a.json")),
            json!({"value": 1}),
            "fetch must not mutate the checkout before the guarded apply"
        );

        let (apply_started_tx, apply_started_rx) = mpsc::channel();
        let (apply_finished_tx, apply_finished_rx) = mpsc::channel();
        let apply_path = local_path.clone();
        let apply_outcome = outcome.clone();
        let apply = std::thread::spawn(move || {
            apply_started_tx.send(()).unwrap();
            let repo = GitRepo::open(&apply_path).unwrap();
            apply_fast_forward(&repo, &apply_outcome, "race").unwrap();
            apply_finished_tx.send(()).unwrap();
        });
        apply_started_rx
            .recv_timeout(Duration::from_secs(1))
            .unwrap();
        assert!(
            apply_finished_rx
                .recv_timeout(Duration::from_millis(100))
                .is_err(),
            "guarded apply must wait for the concurrent scoped write"
        );

        writer_release_tx.send(()).unwrap();
        writer.join().unwrap();
        apply_finished_rx
            .recv_timeout(Duration::from_secs(1))
            .unwrap();
        apply.join().unwrap();

        assert_eq!(
            read_json(&local_path.join("default/core/plugin-configs/plugin-a.json")),
            json!({"value": 2})
        );
    }

    #[tokio::test]
    async fn explicit_conflict_resolution_is_seen_by_profile_reconciliation() {
        let _env = crate::test_support::env_lock().lock().await;
        let tmp = TempDir::new().unwrap();
        let _path = TestPathRootEnvGuard::new(tmp.path());
        let profile_path = crate::paths::profile_dir().unwrap();
        let url = init_bare_origin(&tmp.path().join("origin.git"));
        let repo = GitRepo::init(&profile_path, &url).unwrap();
        write_file(
            &profile_path.join("default/manifest.json"),
            &json!({"version": 1}),
        );
        write_file(
            &profile_path.join("default/core/plugin-configs/plugin-a.json"),
            &json!({"value": 1}),
        );
        write_file(
            &profile_path.join("default/core/plugins.lock.json"),
            &json!({
                "version": 1,
                "plugins": [{
                    "id": "plugin-a",
                    "uid": "plugin-a",
                    "repo_url": "https://example.com/plugin-a.git",
                    "version": "1.0.0"
                }]
            }),
        );
        repo.commit_all("seed", &sig()).unwrap();
        repo.push(None).unwrap();

        let remote_path = tmp.path().join("remote/profile");
        let remote = GitRepo::clone(&url, &remote_path, None).unwrap();
        write_file(
            &remote_path.join("default/core/plugin-configs/plugin-a.json"),
            &json!({"value": 2}),
        );
        write_file(
            &remote_path.join("default/core/plugins.lock.json"),
            &json!({
                "version": 1,
                "plugins": [{
                    "id": "plugin-a",
                    "uid": "plugin-a",
                    "repo_url": "https://example.com/plugin-a.git",
                    "version": "2.0.0"
                }]
            }),
        );
        remote.commit_all("remote state", &sig()).unwrap();
        remote.push(None).unwrap();

        write_file(
            &profile_path.join("default/core/plugin-configs/plugin-a.json"),
            &json!({"value": 3}),
        );
        write_file(
            &profile_path.join("default/core/plugins.lock.json"),
            &json!({
                "version": 1,
                "plugins": [{
                    "id": "plugin-a",
                    "uid": "plugin-a",
                    "repo_url": "https://example.com/plugin-a.git",
                    "version": "1.5.0"
                }]
            }),
        );
        repo.commit_all("local state", &sig()).unwrap();
        let outcome = repo.fetch(None).unwrap();
        assert!(matches!(outcome, PullOutcome::Diverged { .. }));

        write_file(
            &crate::paths::github_auth_path().unwrap(),
            &json!({
                "access_token": "test-token",
                "source": "oauth",
                "scopes": ["repo"]
            }),
        );
        save_sync_target(&SyncTarget {
            repo_url: url,
            auto_created: false,
        })
        .unwrap();

        let manifest: crate::plugins::PluginManifest = toml::from_str(
            r#"
[plugin]
id = "plugin-a"
name = "Plugin A"
description = ""
version = "1.0.0"

[menu]
label = "Plugin A"
items = []
"#,
        )
        .unwrap();
        let mut runtime_context = crate::plugins::config::RuntimeConfigContext::new().unwrap();
        let old_runtime = runtime_context
            .materialize_runtime_config_for_manifest("plugin-a", &manifest)
            .unwrap()
            .unwrap();
        assert_eq!(old_runtime, json!({"value": 3}));

        let mut manager = crate::plugins::PluginManager::new();
        let before_generation = crate::plugins::config::current_profile_config_generation();
        let before_reconciliations = manager.profile_reconciliation_count();
        let service = SyncService::new(crate::paths::plugins_dir().unwrap()).unwrap();
        let result = service
            .resolve_conflicts(vec![
                ConflictChoice {
                    file: "default/core/plugin-configs/plugin-a.json".to_string(),
                    key_path: "value".to_string(),
                    side: Side::Remote,
                },
                ConflictChoice {
                    file: "default/core/plugins.lock.json".to_string(),
                    key_path: "plugins.plugin-a".to_string(),
                    side: Side::Remote,
                },
            ])
            .await
            .unwrap();
        assert!(result.applied_remote);
        assert!(crate::plugins::config::current_profile_config_generation() > before_generation);
        assert_eq!(
            read_json(&profile_path.join("default/core/plugin-configs/plugin-a.json")),
            json!({"value": 2})
        );
        assert_eq!(
            read_json(&profile_path.join("default/core/plugins.lock.json"))["plugins"][0]
                ["version"],
            "2.0.0"
        );

        assert!(manager.reconcile_profile_generation().unwrap());
        assert_eq!(
            manager.profile_reconciliation_count(),
            before_reconciliations + 1
        );
        let new_runtime = runtime_context
            .materialize_runtime_config_for_manifest("plugin-a", &manifest)
            .unwrap()
            .unwrap();
        assert_eq!(new_runtime, json!({"value": 2}));
    }

    #[test]
    fn pull_before_push_merges_independent_remote_changes() {
        let tmp = TempDir::new().unwrap();
        let url = init_bare_origin(&tmp.path().join("origin.git"));
        let alice_path = tmp.path().join("alice/profile");
        let alice = seed_profile_repo(&alice_path, &url);
        let bob_path = tmp.path().join("bob/profile");
        GitRepo::clone(&url, &bob_path, None).unwrap();

        write_file(
            &alice_path.join("default/core/plugin-configs/plugin-a.json"),
            &json!({"value": 2}),
        );
        write_file(
            &alice_path.join("default/core/plugins.lock.json"),
            &json!({
                "version": 1,
                "plugins": [{"id": "plugin-a", "uid": "remote-uid", "version": "2.0.0"}]
            }),
        );
        alice.commit_all("alice", &sig()).unwrap();
        alice.push(None).unwrap();

        write_file(
            &bob_path.join("default/core/plugin-configs/plugin-b.json"),
            &json!({"enabled": true}),
        );
        let before = crate::plugins::config::current_profile_config_generation();
        let output = push_profile_changes(&bob_path, None, "manual push").unwrap();

        assert!(output.conflicts.is_empty());
        assert!(output.pushed);
        assert!(output.applied_remote);
        assert!(crate::plugins::config::current_profile_config_generation() > before);

        let verify_path = tmp.path().join("verify/profile");
        GitRepo::clone(&url, &verify_path, None).unwrap();
        assert_eq!(
            read_json(&verify_path.join("default/core/plugin-configs/plugin-a.json")),
            json!({"value": 2})
        );
        assert_eq!(
            read_json(&verify_path.join("default/core/plugin-configs/plugin-b.json")),
            json!({"enabled": true})
        );
        assert_eq!(
            read_json(&verify_path.join("default/core/plugins.lock.json"))["plugins"][0]["uid"],
            "remote-uid"
        );
    }

    #[test]
    fn pull_before_push_reports_conflicts_without_push_error() {
        let tmp = TempDir::new().unwrap();
        let url = init_bare_origin(&tmp.path().join("origin.git"));
        let alice_path = tmp.path().join("alice/profile");
        let alice = seed_profile_repo(&alice_path, &url);
        let bob_path = tmp.path().join("bob/profile");
        GitRepo::clone(&url, &bob_path, None).unwrap();

        write_file(
            &alice_path.join("default/core/plugin-configs/plugin-a.json"),
            &json!({"value": 2}),
        );
        alice.commit_all("alice", &sig()).unwrap();
        alice.push(None).unwrap();

        write_file(
            &bob_path.join("default/core/plugin-configs/plugin-a.json"),
            &json!({"value": 3}),
        );
        let output = push_profile_changes(&bob_path, None, "manual push").unwrap();

        assert_eq!(output.conflicts.len(), 1);
        assert_eq!(output.conflicts[0].key_path, "value");
        assert!(!output.applied_remote);

        let verify_path = tmp.path().join("verify/profile");
        GitRepo::clone(&url, &verify_path, None).unwrap();
        assert_eq!(
            read_json(&verify_path.join("default/core/plugin-configs/plugin-a.json")),
            json!({"value": 2})
        );
    }
}
