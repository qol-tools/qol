#![cfg(feature = "dev")]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use crate::daemon::{DaemonEvent, EventBus};
use crate::plugins::PluginManager;

use super::state::{PluginCpuState, PluginPidSet};

mod broadcast;
mod filter;
mod pid_collection;
mod rows;

pub(super) fn sample_once(
    state: &Arc<Mutex<PluginCpuState>>,
    plugin_manager: &Arc<Mutex<PluginManager>>,
    history_limit: usize,
) {
    let cpu_percent_window_samples = super::platform::cpu_percent_window_samples().max(1);
    let mut plugin_pids = pid_collection::collect_plugin_pids(plugin_manager);
    let Some(plugin_pids) = filter::filter_monitored_plugins(state, &mut plugin_pids) else {
        return;
    };
    let active_plugins = pid_collection::active_plugin_ids(&plugin_pids);
    let active_pids = pid_collection::active_pids(&plugin_pids);
    let current_cpu_by_pid = pid_collection::current_cpu_by_pid(&active_pids);
    let now = Instant::now();
    let timestamp_ms = now_millis();
    let Ok(mut guard) = state.lock() else {
        return;
    };
    let elapsed = rows::elapsed_seconds(&guard, now);
    guard.last_sample_at = Some(now);
    guard.last_timestamp_ms = timestamp_ms;
    rows::retain_active(&mut guard, &active_pids, &active_plugins);
    sample_rows(
        &mut guard,
        plugin_pids,
        &current_cpu_by_pid,
        elapsed,
        timestamp_ms,
        cpu_percent_window_samples,
        history_limit,
    );
}

fn sample_rows(
    guard: &mut PluginCpuState,
    plugin_pids: HashMap<String, PluginPidSet>,
    current_cpu_by_pid: &HashMap<i32, u64>,
    elapsed: f64,
    timestamp_ms: u64,
    cpu_percent_window_samples: usize,
    history_limit: usize,
) {
    for (plugin_id, pid_set) in plugin_pids {
        rows::sample_plugin_row(
            guard,
            plugin_id,
            pid_set,
            current_cpu_by_pid,
            elapsed,
            timestamp_ms,
            cpu_percent_window_samples,
            history_limit,
        );
    }
}

pub(super) fn broadcast_snapshot(state: &Arc<Mutex<PluginCpuState>>, events: &Arc<EventBus>) {
    let Some((timestamp_ms, plugins)) = broadcast::snapshot_for_broadcast(state) else {
        return;
    };
    events.send(DaemonEvent::PluginCpuSnapshot {
        timestamp_ms,
        plugins,
    });
}

pub(super) fn set_monitored_plugins(state: &Mutex<PluginCpuState>, plugin_ids: Vec<String>) {
    filter::set_monitored_plugins(state, plugin_ids);
}

pub(super) fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}
