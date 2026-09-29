use std::future::Future;
use std::sync::{Arc, Mutex};

use crate::daemon::{DaemonEvent, EventBus};
use crate::plugins::PluginManager;

use super::super::dev_runtime::DevRuntimeService;

pub(in crate::features::plugin_store::server) struct PendingRestart {
    runtime: Arc<DevRuntimeService>,
    events: Arc<EventBus>,
    operation: &'static str,
    finished: bool,
}

impl PendingRestart {
    pub(in crate::features::plugin_store::server) fn begin(
        runtime: Arc<DevRuntimeService>,
        events: Arc<EventBus>,
        operation: &'static str,
    ) -> Result<Self, &'static str> {
        if !runtime.try_mark_restart_pending() {
            return Err("Restart already pending");
        }
        Ok(Self {
            runtime,
            events,
            operation,
            finished: false,
        })
    }

    pub(in crate::features::plugin_store::server) fn fail(&mut self, message: String) {
        log::error!("{} failed: {}", self.operation, message);
        self.finished = true;
        self.runtime.clear_restart_pending();
        self.events
            .send(DaemonEvent::SelfRecompileFailed { message });
    }

    pub(in crate::features::plugin_store::server) async fn run(
        mut self,
        cleanup: impl Future<Output = Result<(), String>>,
        execute: impl FnOnce() -> Result<(), String>,
    ) -> Result<(), String> {
        let result = match cleanup.await {
            Ok(()) => execute(),
            Err(error) => Err(error),
        };
        if let Err(message) = &result {
            self.fail(message.clone());
            return result;
        }
        self.finished = true;
        self.runtime.clear_restart_pending();
        result
    }
}

impl Drop for PendingRestart {
    fn drop(&mut self) {
        if !self.finished {
            self.fail(format!(
                "{} cancelled before process replacement",
                self.operation
            ));
        }
    }
}

pub(in crate::features::plugin_store::server) async fn cleanup_before_restart(
    plugin_manager: Arc<Mutex<PluginManager>>,
) -> Result<(), String> {
    crate::runtime::RuntimeServer::shutdown_peers_and_wait().await?;
    tokio::task::spawn_blocking(move || {
        shutdown_plugin_manager(&plugin_manager);
        verify_plugin_process_leaks()
    })
    .await
    .map_err(|error| format!("Restart plugin cleanup worker failed: {error}"))?
}

fn shutdown_plugin_manager(plugin_manager: &Arc<Mutex<PluginManager>>) {
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
    let report = crate::doctor::fix_single("plugin_process_leaks");
    if !report.failures.is_empty() {
        return Err(format!(
            "plugin process leak cleanup failed: {}",
            report.failures.join("; ")
        ));
    }
    if report.after.has_warnings() || report.after.has_errors() || report.after.has_crashes() {
        return Err(format_plugin_leak_report(&report.after));
    }
    if report.applied > 0 {
        log::warn!(
            "Restart applied {} plugin process leak cleanup fix(es) before restart",
            report.applied
        );
    }
    Ok(())
}

fn format_plugin_leak_report(report: &crate::doctor::Report) -> String {
    report
        .outcomes()
        .filter(|outcome| !matches!(outcome.status, crate::doctor::OutcomeStatus::Ok))
        .map(|outcome| outcome.message.clone())
        .collect::<Vec<_>>()
        .join("; ")
}

#[cfg(test)]
mod tests;
