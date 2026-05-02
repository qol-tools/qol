use crate::plugins::{PluginId, PluginManager};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::broadcast;

const SUPERVISION_INTERVAL: Duration = Duration::from_secs(5);
const MAX_CONSECUTIVE_FAILURES: u32 = 5;

#[derive(Debug, Clone, PartialEq, Eq)]
struct DaemonSnapshot {
    plugin_id: PluginId,
    daemon_pid: Option<u32>,
}

pub fn spawn_supervisor(
    plugin_manager: Arc<Mutex<PluginManager>>,
    shutdown_rx: broadcast::Receiver<()>,
) {
    tokio::spawn(async move {
        run_supervision_loop(plugin_manager, shutdown_rx).await;
    });
}

async fn run_supervision_loop(
    plugin_manager: Arc<Mutex<PluginManager>>,
    mut shutdown_rx: broadcast::Receiver<()>,
) {
    let mut backoff = FailureBackoff::default();
    loop {
        tokio::select! {
            _ = shutdown_rx.recv() => {
                log::info!("Daemon supervisor shutting down");
                return;
            }
            _ = tokio::time::sleep(SUPERVISION_INTERVAL) => {
                supervise_once(&plugin_manager, &mut backoff);
            }
        }
    }
}

fn supervise_once(plugin_manager: &Arc<Mutex<PluginManager>>, backoff: &mut FailureBackoff) {
    let snapshots = snapshot_daemons(plugin_manager);
    let dead_plugins: Vec<PluginId> = snapshots
        .into_iter()
        .filter(|s| !is_daemon_alive(s.daemon_pid))
        .map(|s| s.plugin_id)
        .filter(|id| backoff.can_retry(id))
        .collect();
    if dead_plugins.is_empty() {
        return;
    }

    let mut any_restarted = false;
    for plugin_id in dead_plugins {
        match restart_daemon(plugin_manager, &plugin_id) {
            Ok(()) => {
                backoff.record_success(&plugin_id);
                any_restarted = true;
                log::info!("Restarted dead daemon for plugin {}", plugin_id);
            }
            Err(e) => {
                backoff.record_failure(&plugin_id);
                log::warn!("Failed to restart daemon for plugin {}: {}", plugin_id, e);
            }
        }
    }
    if any_restarted {
        crate::hotkeys::trigger_reload();
    }
}

fn snapshot_daemons(plugin_manager: &Arc<Mutex<PluginManager>>) -> Vec<DaemonSnapshot> {
    let Ok(manager) = plugin_manager.lock() else {
        log::error!("Daemon supervisor: plugin manager lock poisoned");
        return Vec::new();
    };
    manager
        .plugins()
        .filter(|p| p.manifest.daemon.as_ref().is_some_and(|d| d.enabled))
        .map(|p| DaemonSnapshot {
            plugin_id: p.id.clone(),
            daemon_pid: p.daemon_pid(),
        })
        .collect()
}

fn restart_daemon(
    plugin_manager: &Arc<Mutex<PluginManager>>,
    plugin_id: &PluginId,
) -> anyhow::Result<()> {
    let mut manager = plugin_manager
        .lock()
        .map_err(|_| anyhow::anyhow!("plugin manager lock poisoned"))?;
    manager.ensure_plugin_daemon_running(plugin_id.as_str())
}

fn is_daemon_alive(daemon_pid: Option<u32>) -> bool {
    let Some(pid) = daemon_pid else {
        return false;
    };
    crate::process_utils::is_pid_alive(pid as i32)
}

#[derive(Default)]
struct FailureBackoff {
    counts: HashMap<PluginId, u32>,
}

impl FailureBackoff {
    fn can_retry(&self, plugin_id: &PluginId) -> bool {
        self.counts.get(plugin_id).copied().unwrap_or(0) < MAX_CONSECUTIVE_FAILURES
    }

    fn record_failure(&mut self, plugin_id: &PluginId) {
        let entry = self.counts.entry(plugin_id.clone()).or_insert(0);
        *entry += 1;
        if *entry == MAX_CONSECUTIVE_FAILURES {
            log::error!(
                "Daemon supervisor: plugin {} hit {} consecutive failures, suppressing",
                plugin_id,
                MAX_CONSECUTIVE_FAILURES
            );
        }
    }

    fn record_success(&mut self, plugin_id: &PluginId) {
        self.counts.remove(plugin_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pid(id: &str) -> PluginId {
        PluginId::new(id.to_string())
    }

    #[test]
    fn backoff_allows_retries_until_threshold_then_suppresses() {
        let mut backoff = FailureBackoff::default();
        let p = pid("plugin-foo");

        for _ in 0..MAX_CONSECUTIVE_FAILURES {
            assert!(backoff.can_retry(&p), "should allow retry below threshold");
            backoff.record_failure(&p);
        }
        assert!(
            !backoff.can_retry(&p),
            "should suppress after {} failures",
            MAX_CONSECUTIVE_FAILURES
        );
    }

    #[test]
    fn backoff_success_resets_count() {
        let mut backoff = FailureBackoff::default();
        let p = pid("plugin-foo");

        backoff.record_failure(&p);
        backoff.record_failure(&p);
        backoff.record_success(&p);

        for _ in 0..MAX_CONSECUTIVE_FAILURES {
            assert!(backoff.can_retry(&p), "success reset failure count");
            backoff.record_failure(&p);
        }
        assert!(
            !backoff.can_retry(&p),
            "threshold reached again after reset"
        );
    }

    #[test]
    fn backoff_tracks_plugins_independently() {
        let mut backoff = FailureBackoff::default();
        let foo = pid("plugin-foo");
        let bar = pid("plugin-bar");

        for _ in 0..MAX_CONSECUTIVE_FAILURES {
            backoff.record_failure(&foo);
        }
        assert!(!backoff.can_retry(&foo), "foo suppressed");
        assert!(backoff.can_retry(&bar), "bar still retriable");
    }

    #[test]
    fn is_daemon_alive_none_returns_false() {
        assert!(!is_daemon_alive(None));
    }
}
