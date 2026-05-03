use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::super::super::dev_runtime::DevRuntimeService;
use super::super::super::restart::RestartPort;

const RESTART_IDLE_POLL_MS: u64 = 250;

pub(super) fn schedule_self_restart_after_idle(
    plugin_manager: Arc<Mutex<crate::plugins::PluginManager>>,
    runtime: Arc<DevRuntimeService>,
    restart: Arc<dyn RestartPort>,
    repo_root: PathBuf,
    worktree_branch: Option<String>,
    events: Arc<crate::daemon::EventBus>,
) {
    if !runtime.try_mark_restart_pending() {
        return;
    }

    tokio::spawn(async move {
        wait_for_restart_idle(runtime.as_ref()).await;
        let Some(restart_binary) = resolve_restart_binary(
            runtime.as_ref(),
            restart.as_ref(),
            Some(repo_root.as_path()),
        ) else {
            events.send(crate::daemon::DaemonEvent::SelfRecompileFailed {
                message: "Restart binary not found after build".to_string(),
            });
            return;
        };
        exec_restart_after_cleanup(
            plugin_manager,
            runtime.as_ref(),
            restart.as_ref(),
            &restart_binary,
            worktree_branch.as_deref(),
            events.as_ref(),
        );
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
    runtime: &DevRuntimeService,
    restart: &dyn RestartPort,
    worktree_path: Option<&Path>,
) -> Option<PathBuf> {
    let binary = worktree_path
        .map(|wt| restart.binary_at(wt))
        .filter(|p| p.is_file())
        .or_else(|| restart.resolve_restart_binary());

    if binary.is_none() {
        log::error!("Self recompile completed but restart binary could not be resolved");
        runtime.clear_restart_pending();
    }

    binary
}

pub(super) fn resolve_branch_from_path(worktree_path: &Path) -> Option<String> {
    let known = super::super::list_worktrees();
    known
        .into_iter()
        .find(|w| Path::new(&w.path) == worktree_path)
        .map(|w| w.branch)
}

fn exec_restart_after_cleanup(
    plugin_manager: Arc<Mutex<crate::plugins::PluginManager>>,
    runtime: &DevRuntimeService,
    restart: &dyn RestartPort,
    restart_binary: &Path,
    worktree_branch: Option<&str>,
    events: &crate::daemon::EventBus,
) {
    if let Err(message) = cleanup_before_restart(&plugin_manager) {
        log::error!("Self recompile cleanup failed: {}", message);
        events.send(crate::daemon::DaemonEvent::SelfRecompileFailed {
            message: message.clone(),
        });
        runtime.clear_restart_pending();
        return;
    }

    worktree_branch.map_or_else(
        || std::env::remove_var("QOL_DEV_WORKTREE_BRANCH"),
        |branch| std::env::set_var("QOL_DEV_WORKTREE_BRANCH", branch),
    );
    if let Err(error) = restart.exec_restart(restart_binary) {
        log::error!(
            "Self recompile exec restart failed for {}: {}",
            restart_binary.display(),
            error
        );
        runtime.clear_restart_pending();
        std::process::exit(1);
    }
    std::process::exit(0);
}

fn cleanup_before_restart(
    plugin_manager: &Arc<Mutex<crate::plugins::PluginManager>>,
) -> Result<(), String> {
    shutdown_plugin_manager(plugin_manager);
    verify_plugin_process_leaks()
}

fn shutdown_plugin_manager(plugin_manager: &Arc<Mutex<crate::plugins::PluginManager>>) {
    let mut manager = plugin_manager.lock().unwrap_or_else(|poisoned| {
        log::error!(
            "Plugin manager lock poisoned during self restart: {}",
            poisoned
        );
        poisoned.into_inner()
    });
    manager.shutdown();
}

fn verify_plugin_process_leaks() -> Result<(), String> {
    let report = crate::doctor::fix_plugin_process_leaks();
    if !report.failures.is_empty() {
        return Err(format!(
            "plugin process leak cleanup failed: {}",
            report.failures.join("; ")
        ));
    }
    if report.after.has_warnings() || report.after.has_errors() {
        return Err(format_plugin_leak_report(&report.after));
    }
    if report.applied > 0 {
        log::warn!(
            "Self recompile applied {} plugin process leak cleanup fix(es) before restart",
            report.applied
        );
    }
    Ok(())
}

fn format_plugin_leak_report(report: &crate::doctor::Report) -> String {
    report
        .outcomes
        .iter()
        .filter(|outcome| !matches!(outcome.status, crate::doctor::OutcomeStatus::Ok))
        .map(|outcome| outcome.message.clone())
        .collect::<Vec<_>>()
        .join("; ")
}
