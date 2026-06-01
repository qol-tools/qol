use std::path::PathBuf;
use std::sync::Arc;

use super::super::super::dev_runtime::DevRuntimeService;
use super::super::super::types::AppState;
use crate::daemon::{DaemonEvent, EventBus};
use crate::dev;

use super::{result, RecompileResult, SelfRecompileTask};

pub(super) fn queue_self_recompile(
    state: &AppState,
    worktree_path: Option<PathBuf>,
) -> Result<(), &'static str> {
    if !state.runtime.try_start_self_recompile() {
        return Err("Self recompile already in progress");
    }
    log::info!("Developer self recompile requested");

    let repo_root = dev::resolve_qol_tray_self_root(worktree_path.as_deref());
    let branch = resolve_recompile_branch(worktree_path.as_deref(), &repo_root);
    super::super::super::helpers::persist_worktree_branch(branch.as_deref());

    tokio::spawn(run_self_recompile(SelfRecompileTask::from_state(
        state, repo_root, branch,
    )));
    Ok(())
}

struct RecompileGuard {
    runtime: Arc<DevRuntimeService>,
}

impl Drop for RecompileGuard {
    fn drop(&mut self) {
        self.runtime.finish_self_recompile();
    }
}

async fn run_self_recompile(task: SelfRecompileTask) {
    let _guard = RecompileGuard {
        runtime: Arc::clone(&task.runtime),
    };
    let repo_root = task.repo_root.clone();
    let result = spawn_self_recompile(Arc::clone(&task.events), repo_root).await;
    result::handle_recompile_result(task, result);
}

async fn spawn_self_recompile(events: Arc<EventBus>, repo_root: PathBuf) -> RecompileResult {
    tokio::task::spawn_blocking(move || {
        dev::build_qol_tray_self_with_progress(Some(&repo_root), |percent, phase| {
            events.send(DaemonEvent::SelfRecompileProgress { percent, phase });
        })
    })
    .await
}

pub(super) fn resolve_recompile_branch(
    selected_path: Option<&std::path::Path>,
    repo_root: &std::path::Path,
) -> Option<String> {
    selected_path
        .and_then(super::restart_schedule::resolve_branch_from_path)
        .or_else(|| super::restart_schedule::resolve_branch_from_path(repo_root))
}
