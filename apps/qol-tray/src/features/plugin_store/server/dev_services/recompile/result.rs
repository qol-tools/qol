use crate::daemon::{DaemonEvent, EventBus};

use super::{restart_schedule, RecompileResult, SelfRecompileTask};

pub(super) fn handle_recompile_result(task: SelfRecompileTask, result: RecompileResult) {
    match result {
        Ok(build) if build.success => handle_recompile_success(task),
        Ok(build) => handle_recompile_failure(&task.events, build_failure_message(&build.output)),
        Err(error) => handle_recompile_failure(
            &task.events,
            format!("Self recompile worker failed: {}", error),
        ),
    }
}

fn handle_recompile_success(task: SelfRecompileTask) {
    task.events.send(DaemonEvent::SelfRecompileComplete);
    restart_schedule::schedule_self_restart_after_idle(
        task.plugin_manager,
        task.runtime,
        task.restart,
        task.repo_root,
        task.worktree_branch,
        task.events,
    );
}

fn handle_recompile_failure(events: &EventBus, message: String) {
    log::error!("Self recompile failed: {}", message);
    events.send(DaemonEvent::SelfRecompileFailed { message });
}

fn build_failure_message(output: &str) -> String {
    output
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| "Self recompile failed".to_string())
}
