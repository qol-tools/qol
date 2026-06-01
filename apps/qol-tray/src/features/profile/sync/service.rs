use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;
use tokio::sync::Mutex as AsyncMutex;

use super::git_repo::{GitRepo, PullOutcome, SignatureSpec};
use super::state::{
    backup_file_path, build_status, ensure_sync_dirs, filename_string, list_backup_entries,
    load_state_file, load_toggles, now_rfc3339, save_state_file, save_toggles, PullMode,
    SyncStateFile, SyncToggles,
};
use super::types::{
    SyncActionResult, SyncBackupEntry, SyncBackupPreview, SyncConnectRequest, SyncIncident,
    SyncStatus,
};
use crate::features::profile::registry::{
    clear_sync_target, load_sync_target, save_sync_target, SyncTarget,
};

const AUTO_PUSH_INTERVAL_SECS: u64 = 3;
const DEFAULT_REPO_NAME: &str = "qol-tray-profiles";
const GITIGNORE_CONTENTS: &str = "/active\n/sync.json\n*/device/\n";

pub struct SyncService {
    state: Mutex<SyncStateFile>,
    toggles: Mutex<SyncToggles>,
    operation_lock: AsyncMutex<()>,
    http: reqwest::Client,
}

impl SyncService {
    pub fn new(_plugins_dir: PathBuf) -> Result<Self> {
        ensure_sync_dirs().ok();
        let state = load_state_file().unwrap_or_default();
        let toggles = load_toggles().unwrap_or_default();
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

        let repo = if existing_remote_has_content(&self.http, &target, &token).await? {
            clone_remote_via_staging(&target, &repo_path, &token)?
        } else {
            let repo = GitRepo::init(&repo_path, &target.repo_url)?;
            ensure_gitignore(&repo_path)?;
            repo.commit_all(
                "qol-tray: initial commit",
                &SignatureSpec::default_for_app(),
            )?;
            repo.push(Some(&token))?;
            repo
        };

        save_sync_target(&target)?;
        let new_toggles = SyncToggles {
            pull_on_launch: request.pull_on_launch,
            push_on_change: request.push_on_change,
        };
        save_toggles(new_toggles)?;
        *self.toggles_mut() = new_toggles;

        let head = repo.head_sha()?;
        let mut state = self.state_mut();
        state.head_sha = head;
        state.last_sync_at = Some(now_rfc3339());
        state.last_error = None;
        state.incident = None;
        save_state_file(&state)?;
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
        let git_dir = repo_path.join(".git");
        if git_dir.exists() {
            std::fs::remove_dir_all(&git_dir).ok();
        }
        let mut state = self.state_mut();
        *state = SyncStateFile::default();
        save_state_file(&state)?;
        let cleared = state.clone();
        drop(state);
        Ok(SyncActionResult {
            message: "Cloud sync disconnected".to_string(),
            applied_remote: false,
            status: self.build_status_with(&cleared, None),
        })
    }

    pub async fn manual_pull(&self) -> Result<SyncActionResult> {
        self.do_pull(PullMode::Manual).await
    }

    pub async fn pull_on_launch(&self) -> Result<SyncActionResult> {
        if !self.snapshot_toggles().pull_on_launch {
            return self.noop_result("Pull on launch disabled");
        }
        self.do_pull(PullMode::Launch).await
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

    pub async fn acknowledge_incident(&self) -> Result<SyncActionResult> {
        let _operation = self.operation_lock.lock().await;
        let mut state = self.state_mut();
        state.incident = None;
        state.last_error = None;
        save_state_file(&state)?;
        let cleared = state.clone();
        drop(state);
        Ok(SyncActionResult {
            message: "Incident acknowledged".to_string(),
            applied_remote: false,
            status: self.build_status_with(&cleared, load_sync_target().ok().flatten().as_ref()),
        })
    }

    pub fn open_backups_dir(&self) -> Result<()> {
        ensure_sync_dirs()?;
        let dir = crate::paths::sync_backups_dir()?;
        super::platform::open_dir(&dir)
    }

    pub fn open_backup_file(&self, file_name: &str) -> Result<()> {
        let path = backup_file_path(file_name)?;
        if !path.exists() {
            anyhow::bail!("backup not found");
        }
        super::platform::open_path(&path)
    }

    pub fn list_backups(&self) -> Result<Vec<SyncBackupEntry>> {
        let dir = crate::paths::sync_backups_dir().ok();
        Ok(list_backup_entries(dir.as_deref()))
    }

    pub fn preview_backup(&self, file_name: &str) -> Result<SyncBackupPreview> {
        let path = backup_file_path(file_name)?;
        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("read backup {}", path.display()))?;
        Ok(SyncBackupPreview {
            file_name: filename_string(&path),
            content,
        })
    }

    async fn do_pull(&self, mode: PullMode) -> Result<SyncActionResult> {
        let _operation = self.operation_lock.lock().await;
        let Some(target) = load_sync_target()? else {
            return self.noop_result("Sync not configured");
        };
        let token = require_github_token()?;
        let repo_path = crate::paths::profile_dir()?;
        let repo_url = target.repo_url.clone();
        let pulled =
            tokio::task::spawn_blocking(move || -> Result<(PullOutcome, Option<String>)> {
                let repo = match GitRepo::open(&repo_path) {
                    Ok(repo) => repo,
                    Err(_) => GitRepo::clone(&repo_url, &repo_path, Some(&token))?,
                };
                let outcome = repo.pull(Some(&token))?;
                let head = repo.head_sha()?;
                Ok((outcome, head))
            })
            .await
            .context("join sync pull task")?;
        let (outcome, head) = match pulled {
            Ok(value) => value,
            Err(error) => return self.persisted_error(error, Some(&target)),
        };
        let (message, applied) = match &outcome {
            PullOutcome::AlreadyUpToDate => ("Already up to date".to_string(), false),
            PullOutcome::FastForwarded { .. } => ("Pulled changes from remote".to_string(), true),
            PullOutcome::Diverged { .. } => ("Local and remote diverged".to_string(), false),
        };
        let mut state = self.state_mut();
        if let PullOutcome::Diverged { local, remote } = &outcome {
            state.incident = Some(SyncIncident {
                kind: mode.incident_kind(),
                message: format!(
                    "Local {} differs from remote {}",
                    short_sha(local),
                    short_sha(remote)
                ),
                backup_file: None,
                created_at: now_rfc3339(),
            });
        } else {
            state.incident = None;
            state.last_error = None;
        }
        state.head_sha = head;
        state.last_sync_at = Some(now_rfc3339());
        save_state_file(&state)?;
        let saved = state.clone();
        drop(state);
        Ok(SyncActionResult {
            message,
            applied_remote: applied,
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
        let reason_owned = reason.to_string();
        let pushed = tokio::task::spawn_blocking(move || -> Result<(bool, Option<String>)> {
            let repo = GitRepo::open(&repo_path)?;
            ensure_gitignore(&repo_path)?;
            let commit = repo.commit_all(&reason_owned, &SignatureSpec::default_for_app())?;
            repo.push(Some(&token))?;
            let head = repo.head_sha()?;
            Ok((commit.is_some(), head))
        })
        .await
        .context("join sync push task")?;
        let (committed, head) = match pushed {
            Ok(value) => value,
            Err(error) => return self.persisted_error(error, Some(&target)),
        };
        let message = if committed {
            "Pushed changes to remote".to_string()
        } else {
            "Nothing to push".to_string()
        };
        let mut state = self.state_mut();
        state.head_sha = head;
        state.last_sync_at = Some(now_rfc3339());
        state.last_error = None;
        save_state_file(&state)?;
        let saved = state.clone();
        drop(state);
        Ok(SyncActionResult {
            message,
            applied_remote: false,
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
        save_state_file(&state)?;
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
        let backups_dir = crate::paths::sync_backups_dir().ok();
        let backup_files = list_backup_entries(backups_dir.as_deref());
        build_status(
            state,
            target,
            toggles,
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
    super::promote::promote_allowlisted_clone(staging, profile_dir)?;
    super::promote::promote_clone_git_dir(staging, profile_dir)?;
    GitRepo::open(profile_dir).context("open promoted profile repo")
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
    std::fs::write(&path, GITIGNORE_CONTENTS).with_context(|| format!("write {}", path.display()))
}

fn short_sha(sha: &str) -> String {
    sha.chars().take(7).collect()
}
