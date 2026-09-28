use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::super::super::dev_runtime::DevRuntimeService;
use super::super::super::restart::{cleanup_before_restart, PendingRestart, RestartPort};

const RESTART_IDLE_POLL_MS: u64 = 250;

pub(super) fn schedule_self_restart_after_idle(
    plugin_manager: Arc<Mutex<crate::plugins::PluginManager>>,
    runtime: Arc<DevRuntimeService>,
    restart: Arc<dyn RestartPort>,
    repo_root: PathBuf,
    worktree_branch: Option<String>,
    events: Arc<crate::daemon::EventBus>,
) {
    let Ok(mut pending) = PendingRestart::begin(runtime.clone(), events, "Self recompile restart")
    else {
        return;
    };

    tokio::spawn(async move {
        wait_for_restart_idle(runtime.as_ref()).await;
        let Some(restart_binary) =
            resolve_restart_binary(restart.as_ref(), Some(repo_root.as_path()))
        else {
            pending.fail("Restart binary not found after build".to_string());
            return;
        };
        let staging_root = crate::paths::default_workspace_root().unwrap_or(repo_root.clone());
        let staged = match restart.stage_restart_binary(&staging_root, &restart_binary) {
            Ok(staged) => staged,
            Err(message) => {
                pending.fail(format!("Self recompile runtime staging failed: {message}"));
                return;
            }
        };
        exec_restart_after_cleanup(
            plugin_manager,
            pending,
            restart.as_ref(),
            &staging_root,
            &staged,
            worktree_branch.as_deref(),
        )
        .await;
    });
}

async fn wait_for_restart_idle(runtime: &DevRuntimeService) {
    loop {
        if restart_idle(runtime) {
            return;
        }

        tokio::time::sleep(Duration::from_millis(RESTART_IDLE_POLL_MS)).await;
    }
}

fn restart_idle(runtime: &DevRuntimeService) -> bool {
    !runtime.build_in_progress()
        && !runtime.any_mock_target_running()
        && !runtime.self_recompile_in_progress()
}

fn resolve_restart_binary(
    restart: &dyn RestartPort,
    worktree_path: Option<&Path>,
) -> Option<PathBuf> {
    worktree_path
        .map(|wt| restart.binary_at(wt))
        .filter(|p| p.is_file())
        .or_else(|| restart.resolve_restart_binary())
}

pub(super) fn resolve_branch_from_path(worktree_path: &Path) -> Option<String> {
    let known = super::super::list_worktrees();
    known
        .into_iter()
        .find(|w| w.path == worktree_path)
        .map(|w| w.branch)
}

async fn exec_restart_after_cleanup(
    plugin_manager: Arc<Mutex<crate::plugins::PluginManager>>,
    pending: PendingRestart,
    restart: &dyn RestartPort,
    staging_root: &Path,
    staged: &qol_dev_build::tray::StagedRuntimeGeneration,
    worktree_branch: Option<&str>,
) {
    let _ = pending
        .run(cleanup_before_restart(plugin_manager), || {
            exec_staged_restart(restart, staging_root, staged, worktree_branch)
        })
        .await;
}

fn exec_staged_restart(
    restart: &dyn RestartPort,
    staging_root: &Path,
    staged: &qol_dev_build::tray::StagedRuntimeGeneration,
    worktree_branch: Option<&str>,
) -> Result<(), String> {
    if let Ok(config_dir) = crate::paths::shared_config_dir() {
        let env = crate::installer::boot_environment::default_boot_environment();
        let lister = crate::dev::boot_contract::GitWorktreeLister;
        let probe = crate::dev::boot_contract::FsBinaryProbe;
        let _ = crate::dev::boot_contract::set_selected_worktree(
            env.as_ref(),
            &config_dir,
            worktree_branch,
            &lister,
            &probe,
        );
    }
    worktree_branch.map_or_else(
        || std::env::remove_var("QOL_DEV_WORKTREE_BRANCH"),
        |branch| std::env::set_var("QOL_DEV_WORKTREE_BRANCH", branch),
    );
    let current = std::env::current_exe()
        .ok()
        .map(qol_conventions::artifact::normalized_executable);
    let mut protected = vec![staged.executable()];
    if let Some(current) = current.as_deref() {
        protected.push(current);
    }
    if let Err(error) = qol_dev_build::tray::prune_runtime_generations(staging_root, &protected) {
        log::warn!("Self recompile runtime prune failed: {error}");
    }
    restart.exec_restart(staged.executable()).map_err(|error| {
        format!(
            "Self recompile exec restart failed for {}: {}",
            staged.executable().display(),
            error
        )
    })?;
    std::process::exit(0);
}
