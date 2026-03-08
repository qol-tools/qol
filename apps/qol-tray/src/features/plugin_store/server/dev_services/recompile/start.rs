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

    let branch = worktree_path
        .as_deref()
        .and_then(super::restart_schedule::resolve_branch_from_path);
    super::super::super::helpers::persist_worktree_branch(branch.as_deref());

    tokio::spawn(run_self_recompile(SelfRecompileTask::from_state(
        state,
        worktree_path,
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
    let worktree_path = task.worktree_path.clone();
    let result = spawn_self_recompile(Arc::clone(&task.events), worktree_path).await;
    result::handle_recompile_result(task, result);
}

async fn spawn_self_recompile(
    events: Arc<EventBus>,
    worktree_path: Option<PathBuf>,
) -> RecompileResult {
    tokio::task::spawn_blocking(move || {
        dev::build_qol_tray_self_with_progress(worktree_path.as_deref(), |percent, phase| {
            events.send(DaemonEvent::SelfRecompileProgress { percent, phase });
        })
    })
    .await
}
