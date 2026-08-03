//! Headless profile sync (`qol sync`).
//!
//! Thin CLI adapter over the shared `qol-profile-sync` engine. The tray's
//! `SyncService` and this command drive one profile store, one repo layout,
//! one conflict model, and one state-file format: on conflict the local data
//! is kept, a local+remote snapshot backup is written into the tracked
//! backups dir, and nothing is pushed.
//!
//! A running tray owns the profile store: its config guards and generation
//! counter cannot be joined from another process, so when the tray API answers
//! this command delegates pull and push to it. Only with no tray running does
//! the CLI drive the engine itself, under the shared cross-process sync lock.

mod scope;

use anyhow::{anyhow, bail, Context, Result};
use qol_profile_sync::{
    backup_file_path, build_status, list_backup_entries, load_state_file, load_sync_target,
    load_toggles, reconcile, repair_profile_schema, save_state_file, write_conflict_backup,
    GitRepo, PullOutcome, ResolvableConflict, SignatureSpec, SyncIncident, SyncIncidentKind,
    SyncLock, SyncPaths, SyncStateFile, SyncStatus, SyncTarget,
};
use serde_json::Value;
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::Path;

pub(crate) fn run(args: &[OsString]) -> Result<()> {
    if !args.is_empty() {
        bail!("usage: qol sync");
    }
    match sync_profile()? {
        SyncOutcome::NotConfigured => {
            bail!("profile sync is not configured; connect it from qol-tray settings first")
        }
        SyncOutcome::Conflicts { message, status } => {
            let backup = status
                .incident
                .as_ref()
                .and_then(|incident| incident.backup_file.as_ref())
                .cloned()
                .unwrap_or_default();
            let paths = SyncPaths::new(scope::profile_dir()?);
            bail!(
                "profile sync: {message}; local data was kept and a conflict backup was written to {}",
                backup_file_path(&paths, &backup)
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|_| backup)
            )
        }
        SyncOutcome::Synced {
            message, status, ..
        } => {
            println!("profile sync: {message}");
            if let Some(url) = status.repo_url.as_deref() {
                println!("repository: {url}");
            }
            if let Some(sha) = status.head_sha.as_deref() {
                println!("head: {}", short_sha(sha));
            }
            Ok(())
        }
    }
}

/// Structured output for `qol sync --json`, mirroring the tray's
/// `SyncActionResult` shape.
pub(crate) fn run_json() -> Result<Value> {
    let (message, applied_remote, status) = match sync_profile()? {
        SyncOutcome::NotConfigured => {
            let state = load_state_file(&sync_paths()?)?;
            let status = build_status_for(&state, None);
            ("Sync not configured".to_string(), false, status)
        }
        SyncOutcome::Conflicts { message, status } => (message, false, status),
        SyncOutcome::Synced {
            message,
            applied_remote,
            status,
        } => (message, applied_remote, status),
    };
    Ok(serde_json::json!({
        "message": message,
        "applied_remote": applied_remote,
        "status": status,
    }))
}

#[derive(Debug)]
enum SyncOutcome {
    NotConfigured,
    Conflicts {
        message: String,
        status: SyncStatus,
    },
    Synced {
        message: String,
        applied_remote: bool,
        status: SyncStatus,
    },
}

fn sync_paths() -> Result<SyncPaths> {
    Ok(SyncPaths::new(scope::profile_dir()?))
}

fn build_status_for(state: &SyncStateFile, target: Option<&SyncTarget>) -> SyncStatus {
    let paths = sync_paths().ok();
    let backups_dir = paths.as_ref().map(|paths| paths.backups_dir());
    let toggles = paths
        .as_ref()
        .and_then(|paths| load_toggles(paths).ok())
        .unwrap_or_default();
    build_status(
        state,
        target,
        toggles,
        load_github_credential().is_some(),
        backups_dir.as_deref(),
        &list_backup_entries(backups_dir.as_deref()),
    )
}

fn sync_profile() -> Result<SyncOutcome> {
    let paths = sync_paths()?;
    let Some(target) = load_sync_target(paths.profile_root())? else {
        return Ok(SyncOutcome::NotConfigured);
    };
    if scope::is_host_store() && crate::dev_server::api_port_open() {
        return sync_through_host(&target);
    }
    let token = require_github_token()?;
    let repo_path = paths.profile_root().to_path_buf();
    let _sync_lock = SyncLock::acquire(&paths.lock_path())?;
    let repo = GitRepo::open(&repo_path).map_err(|error| {
        anyhow!(
            "profile sync repo is missing or unreadable at {}: {error:#}; reconnect sync from qol-tray settings",
            repo_path.display()
        )
    })?;
    ensure_gitignore(&repo_path)?;

    if repair_profile_schema(&repo_path)? {
        repo.commit_all("repair profile schema", &SignatureSpec::default_for_cli())?;
    }
    repo.commit_all("manual sync", &SignatureSpec::default_for_cli())?;

    let outcome = repo.fetch(Some(&token))?;
    let mut applied_remote = false;
    if matches!(outcome, PullOutcome::FastForwarded { .. }) {
        repo.apply_pull(&outcome)?;
        applied_remote = true;
    }
    let mut conflicts = Vec::new();
    if matches!(outcome, PullOutcome::Diverged { .. }) {
        let merge = reconcile(&repo)?;
        if merge.conflicts.is_empty() {
            apply_merged_profile(&repo, &repo_path, &merge.merged)?;
            repo.commit_all("merge remote changes", &SignatureSpec::default_for_cli())?;
            applied_remote = true;
        } else {
            conflicts = merge.conflicts;
        }
    }

    let mut state = load_state_file(&paths)?;
    if conflicts.is_empty() {
        state.conflicts.clear();
        state.incident = None;
        state.last_error = None;
    } else {
        let (local, remote) = match &outcome {
            PullOutcome::Diverged { local, remote } => (local.clone(), remote.clone()),
            _ => (String::new(), String::new()),
        };
        let backup_file = write_conflict_backup_for(&repo, &paths)?;
        state.incident = Some(SyncIncident {
            kind: SyncIncidentKind::Conflict,
            message: format!(
                "{} setting(s) differ (local {} vs remote {})",
                conflicts.len(),
                short_sha(&local),
                short_sha(&remote)
            ),
            backup_file: Some(backup_file),
            created_at: qol_profile_sync::now_rfc3339(),
        });
        state.conflicts = conflicts
            .into_iter()
            .map(|conflict| ResolvableConflict {
                file: conflict.file,
                plugin: conflict.plugin,
                key_path: conflict.key_path,
                local: conflict.local,
                remote: conflict.remote,
                local_edited: None,
                remote_edited: None,
            })
            .collect();
        state.last_error = None;
    }
    let head = repo.head_sha()?;
    state.head_sha = head;
    state.last_sync_at = Some(qol_profile_sync::now_rfc3339());
    save_state_file(&paths, &state)?;

    if !state.conflicts.is_empty() {
        let message = format!("{} setting(s) need review", state.conflicts.len());
        let status = build_status_for(&state, Some(&target));
        return Ok(SyncOutcome::Conflicts { message, status });
    }

    let remote_before = repo.remote_oid().ok();
    if let Err(error) = repo.push(Some(&token)) {
        state.last_error = Some(format!("{error:#}"));
        save_state_file(&paths, &state)?;
        return Err(error).context("push to remote");
    }
    let pushed = remote_before != repo.remote_oid().ok();

    let message = if pushed {
        "Pushed changes to remote".to_string()
    } else if applied_remote {
        "Pulled changes from remote".to_string()
    } else {
        "Nothing to push".to_string()
    };
    let status = build_status_for(&state, Some(&target));
    Ok(SyncOutcome::Synced {
        message,
        applied_remote,
        status,
    })
}

fn sync_through_host(target: &SyncTarget) -> Result<SyncOutcome> {
    let paths = sync_paths()?;
    let (pull_message, applied_remote) = post_sync_action("/api/sync/pull")?;
    let state = load_state_file(&paths)?;
    if !state.conflicts.is_empty() {
        return Ok(SyncOutcome::Conflicts {
            message: pull_message,
            status: build_status_for(&state, Some(target)),
        });
    }

    let (push_message, _) = post_sync_action("/api/sync/push")?;
    let state = load_state_file(&paths)?;
    Ok(SyncOutcome::Synced {
        message: push_message,
        applied_remote,
        status: build_status_for(&state, Some(target)),
    })
}

fn post_sync_action(route: &str) -> Result<(String, bool)> {
    let response = crate::dev_server::post_api_json(route, "{}").with_context(|| {
        format!("qol-tray owns the running profile store, so {route} runs through it")
    })?;
    let value: Value =
        serde_json::from_str(&response).with_context(|| format!("parse {route} response"))?;
    let message = value["message"].as_str().unwrap_or_default().to_string();
    Ok((message, value["applied_remote"].as_bool().unwrap_or(false)))
}

fn apply_merged_profile(
    repo: &GitRepo,
    repo_path: &Path,
    merged: &BTreeMap<String, Value>,
) -> Result<()> {
    repo.reset_to_remote()?;
    write_merged_profile(repo_path, merged)
}

fn write_merged_profile(repo_path: &Path, merged: &BTreeMap<String, Value>) -> Result<()> {
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

fn ensure_gitignore(repo_path: &Path) -> Result<()> {
    let path = repo_path.join(".gitignore");
    if path.exists() {
        return Ok(());
    }
    std::fs::write(&path, qol_profile_sync::GITIGNORE_CONTENTS)
        .with_context(|| format!("write {}", path.display()))
}

fn write_conflict_backup_for(repo: &GitRepo, paths: &SyncPaths) -> Result<String> {
    let local = match repo.local_oid()? {
        Some(oid) => repo.snapshot_json_at(oid, qol_profile_sync::mergeable_path)?,
        None => BTreeMap::new(),
    };
    let remote = repo
        .remote_oid()
        .ok()
        .map(|oid| repo.snapshot_json_at(oid, qol_profile_sync::mergeable_path))
        .transpose()?
        .unwrap_or_default();
    write_conflict_backup(
        paths,
        &serde_json::json!({ "local": local, "remote": remote }),
    )
}

fn short_sha(sha: &str) -> String {
    sha.chars().take(7).collect()
}

fn require_github_token() -> Result<String> {
    load_github_credential()
        .context("GitHub account is not connected; connect profile sync from qol-tray settings")
}

/// Reads the same credential files the tray uses, newest format first and
/// rejecting symlinked credential files.
fn load_github_credential() -> Option<String> {
    #[derive(serde::Deserialize)]
    struct GitHubCredentialRecord {
        access_token: String,
    }
    let auth_path = scope::github_auth_path().ok()?;
    let content = read_regular_file(&auth_path)?;
    if let Ok(record) = serde_json::from_str::<GitHubCredentialRecord>(&content) {
        if !record.access_token.trim().is_empty() {
            return Some(record.access_token);
        }
    }
    let legacy_path = scope::github_token_path().ok()?;
    let token = read_regular_file(&legacy_path)?;
    let token = token.trim();
    if token.is_empty() {
        return None;
    }
    Some(token.to_string())
}

fn read_regular_file(path: &Path) -> Option<String> {
    let metadata = std::fs::symlink_metadata(path).ok()?;
    if metadata.file_type().is_symlink() {
        return None;
    }
    std::fs::read_to_string(path).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use git2::Repository;
    use qol_profile_sync::SyncHealth;
    use serde_json::json;
    use std::path::Path;
    use tempfile::TempDir;

    // Reuse the path-root lock from scope.rs so every env-mutating sync test
    // serializes against each other.
    fn init_bare_origin(dir: &Path) -> String {
        Repository::init_bare(dir).unwrap();
        let normalized = dir.display().to_string().replace('\\', "/");
        format!("file:///{}", normalized.trim_start_matches('/'))
    }

    fn write_file(path: &Path, contents: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
    }

    fn sig() -> SignatureSpec {
        SignatureSpec {
            name: "Tester".to_string(),
            email: "tester@example.com".to_string(),
        }
    }

    /// Seeds a remote profile repo and clones it into the profile dir under a
    /// temp path root, so `sync_profile` operates on a real local store.
    fn seed_environment(tmp: &Path, url: &str) -> EnvGuard {
        let remote_path = tmp.join("remote/profile");
        let remote = GitRepo::init(&remote_path, url).unwrap();
        write_file(
            &remote_path.join("default/manifest.json"),
            "{\n  \"version\": 1\n}\n",
        );
        write_file(
            &remote_path.join("default/core/plugin-configs/plugin-a.json"),
            "{\n  \"value\": 1\n}\n",
        );
        remote.commit_all("seed", &sig()).unwrap();
        remote.push(None).unwrap();

        let root = tmp.join("root");
        let guard = EnvGuard::new(&root);
        std::fs::create_dir_all(scope::profile_dir().unwrap()).unwrap();
        GitRepo::clone(url, &scope::profile_dir().unwrap(), None).unwrap();
        write_file(
            &scope::github_auth_path().unwrap(),
            "{\n  \"access_token\": \"test-token\",\n  \"source\": \"oauth\",\n  \"scopes\": [\"repo\"]\n}\n",
        );
        qol_profile_sync::save_sync_target(
            &scope::profile_dir().unwrap(),
            &SyncTarget {
                repo_url: url.to_string(),
                auto_created: false,
            },
        )
        .unwrap();
        guard
    }

    struct EnvGuard {
        previous: Option<std::ffi::OsString>,
    }

    impl EnvGuard {
        fn new(root: &Path) -> Self {
            let previous = std::env::var_os("QOL_TRAY_TEST_PATH_ROOT");
            std::env::set_var("QOL_TRAY_TEST_PATH_ROOT", root);
            Self { previous }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            if let Some(previous) = &self.previous {
                std::env::set_var("QOL_TRAY_TEST_PATH_ROOT", previous);
                return;
            }
            std::env::remove_var("QOL_TRAY_TEST_PATH_ROOT");
        }
    }

    fn read_json(path: &Path) -> Value {
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
    }

    #[test]
    fn sync_is_noop_when_not_configured() {
        let tmp = TempDir::new().unwrap();
        let _lock = scope::ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let _guard = EnvGuard::new(&tmp.path().join("root"));
        assert!(matches!(
            sync_profile().unwrap(),
            SyncOutcome::NotConfigured
        ));
    }

    #[test]
    fn sync_pushes_local_changes_and_pulls_remote() {
        let tmp = TempDir::new().unwrap();
        let _lock = scope::ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let url = init_bare_origin(&tmp.path().join("origin.git"));
        let _guard = seed_environment(tmp.path(), &url);

        // Local change: new plugin config.
        write_file(
            &scope::profile_dir()
                .unwrap()
                .join("default/core/plugin-configs/plugin-b.json"),
            &serde_json::to_string_pretty(&json!({"enabled": true})).unwrap(),
        );

        let outcome = sync_profile().unwrap();
        let SyncOutcome::Synced {
            message, status, ..
        } = outcome
        else {
            panic!("expected synced outcome, got {outcome:?}");
        };
        assert_eq!(message, "Pushed changes to remote");
        assert_eq!(status.health, SyncHealth::Healthy);
        assert_eq!(status.conflict_count, 0);

        // A fresh clone of the origin sees both the local push and the seed.
        let verify = tmp.path().join("verify/profile");
        GitRepo::clone(&url, &verify, None).unwrap();
        assert_eq!(
            read_json(&verify.join("default/core/plugin-configs/plugin-a.json")),
            json!({"value": 1})
        );
        assert_eq!(
            read_json(&verify.join("default/core/plugin-configs/plugin-b.json")),
            json!({"enabled": true})
        );
    }

    #[test]
    fn sync_auto_merges_independent_remote_changes() {
        let tmp = TempDir::new().unwrap();
        let _lock = scope::ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let url = init_bare_origin(&tmp.path().join("origin.git"));
        let _guard = seed_environment(tmp.path(), &url);

        // Remote changes plugin-a; local adds plugin-b. No field clash.
        let remote_path = tmp.path().join("remote/profile");
        write_file(
            &remote_path.join("default/core/plugin-configs/plugin-a.json"),
            &serde_json::to_string_pretty(&json!({"value": 2})).unwrap(),
        );
        let remote = GitRepo::open(&remote_path).unwrap();
        remote.commit_all("remote state", &sig()).unwrap();
        remote.push(None).unwrap();

        write_file(
            &scope::profile_dir()
                .unwrap()
                .join("default/core/plugin-configs/plugin-b.json"),
            &serde_json::to_string_pretty(&json!({"enabled": true})).unwrap(),
        );

        let outcome = sync_profile().unwrap();
        let SyncOutcome::Synced {
            message, status, ..
        } = outcome
        else {
            panic!("expected synced outcome, got {outcome:?}");
        };
        assert_eq!(message, "Pushed changes to remote");
        assert_eq!(status.conflict_count, 0);
        assert_eq!(
            read_json(
                &scope::profile_dir()
                    .unwrap()
                    .join("default/core/plugin-configs/plugin-a.json")
            ),
            json!({"value": 2}),
            "remote field must be merged in"
        );

        let verify = tmp.path().join("verify/profile");
        GitRepo::clone(&url, &verify, None).unwrap();
        assert_eq!(
            read_json(&verify.join("default/core/plugin-configs/plugin-b.json")),
            json!({"enabled": true}),
            "local field must be pushed"
        );
    }

    #[test]
    fn sync_conflicts_keep_local_data_and_write_backup() {
        let tmp = TempDir::new().unwrap();
        let _lock = scope::ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let url = init_bare_origin(&tmp.path().join("origin.git"));
        let _guard = seed_environment(tmp.path(), &url);

        // Both sides edit the same field.
        let remote_path = tmp.path().join("remote/profile");
        write_file(
            &remote_path.join("default/core/plugin-configs/plugin-a.json"),
            &serde_json::to_string_pretty(&json!({"value": 2})).unwrap(),
        );
        let remote = GitRepo::open(&remote_path).unwrap();
        remote.commit_all("remote state", &sig()).unwrap();
        remote.push(None).unwrap();

        write_file(
            &scope::profile_dir()
                .unwrap()
                .join("default/core/plugin-configs/plugin-a.json"),
            &serde_json::to_string_pretty(&json!({"value": 3})).unwrap(),
        );

        let outcome = sync_profile().unwrap();
        let SyncOutcome::Conflicts { message, status } = outcome else {
            panic!("expected conflicts outcome, got {outcome:?}");
        };
        assert_eq!(message, "1 setting(s) need review");
        assert_eq!(status.conflict_count, 1);
        assert_eq!(status.health, SyncHealth::Attention);

        // Local data kept, remote not applied.
        assert_eq!(
            read_json(
                &scope::profile_dir()
                    .unwrap()
                    .join("default/core/plugin-configs/plugin-a.json")
            ),
            json!({"value": 3})
        );

        // Conflict backup written and recorded in state.
        let state = load_state_file(&SyncPaths::new(scope::profile_dir().unwrap())).unwrap();
        let incident = state.incident.unwrap();
        let backup_name = incident.backup_file.unwrap();
        let backups_dir = SyncPaths::new(scope::profile_dir().unwrap()).backups_dir();
        let backup = read_json(&backups_dir.join(&backup_name));
        assert_eq!(
            backup["local"]["default/core/plugin-configs/plugin-a.json"]["value"],
            3
        );
        assert_eq!(
            backup["remote"]["default/core/plugin-configs/plugin-a.json"]["value"],
            2
        );

        // Origin holds the remote change but not the local one: nothing was
        // pushed while the conflict is pending.
        let verify = tmp.path().join("verify/profile");
        GitRepo::clone(&url, &verify, None).unwrap();
        assert_eq!(
            read_json(&verify.join("default/core/plugin-configs/plugin-a.json")),
            json!({"value": 2})
        );
    }

    #[test]
    fn sync_reports_pushed_when_a_preexisting_local_commit_is_transferred() {
        let tmp = TempDir::new().unwrap();
        let _lock = scope::ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let url = init_bare_origin(&tmp.path().join("origin.git"));
        let _guard = seed_environment(tmp.path(), &url);

        sync_profile().unwrap();

        let local = GitRepo::open(&scope::profile_dir().unwrap()).unwrap();
        write_file(
            &scope::profile_dir()
                .unwrap()
                .join("default/core/plugin-configs/plugin-a.json"),
            &serde_json::to_string_pretty(&json!({"value": 2})).unwrap(),
        );
        local.commit_all("pending local commit", &sig()).unwrap();

        let outcome = sync_profile().unwrap();
        let SyncOutcome::Synced { message, .. } = outcome else {
            panic!("expected synced outcome, got {outcome:?}");
        };
        assert_eq!(
            message, "Pushed changes to remote",
            "a pending local commit must be reported as pushed"
        );

        let verify = tmp.path().join("verify/profile");
        GitRepo::clone(&url, &verify, None).unwrap();
        assert_eq!(
            read_json(&verify.join("default/core/plugin-configs/plugin-a.json")),
            json!({"value": 2}),
            "the pending commit must reach the origin"
        );
    }

    #[test]
    fn sync_reports_nothing_to_push_when_local_and_remote_are_in_sync() {
        let tmp = TempDir::new().unwrap();
        let _lock = scope::ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let url = init_bare_origin(&tmp.path().join("origin.git"));
        let _guard = seed_environment(tmp.path(), &url);

        sync_profile().unwrap();

        let outcome = sync_profile().unwrap();
        let SyncOutcome::Synced { message, .. } = outcome else {
            panic!("expected synced outcome, got {outcome:?}");
        };
        assert_eq!(message, "Nothing to push");
    }

    #[test]
    fn run_json_returns_structured_status_when_not_configured() {
        let tmp = TempDir::new().unwrap();
        let _lock = scope::ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let _guard = EnvGuard::new(&tmp.path().join("root"));

        let value = run_json().unwrap();
        assert_eq!(value["message"], "Sync not configured");
        assert_eq!(value["applied_remote"], false);
        assert_eq!(value["status"]["configured"], false);
        assert_eq!(value["status"]["health"], "not_configured");
    }
}
