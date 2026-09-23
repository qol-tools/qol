#![cfg(feature = "dev")]

use serde::Serialize;
use std::cmp::Ordering;
use std::sync::Mutex;
use std::time::Duration;

use super::sampling::now_millis;
use super::state::PluginCpuState;

#[derive(Debug, Clone, Serialize, Default)]
pub struct PluginCpuPoint {
    pub timestamp_ms: u64,
    pub cpu_percent: f64,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct PluginCpuEntry {
    pub plugin_id: String,
    pub daemon_pid: Option<u32>,
    pub action_pids: Vec<u32>,
    pub cpu_percent: f64,
    pub cpu_seconds_total: f64,
    pub history: Vec<PluginCpuPoint>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub(crate) struct PluginCpuResponse {
    timestamp_ms: u64,
    sample_interval_ms: u64,
    history_limit: usize,
    plugins: Vec<PluginCpuEntry>,
}

pub(super) fn build_snapshot(
    state: &Mutex<PluginCpuState>,
    sample_interval: Duration,
    history_limit: usize,
) -> PluginCpuResponse {
    PluginCpuResponse {
        timestamp_ms: snapshot_timestamp(state),
        sample_interval_ms: sample_interval.as_millis() as u64,
        history_limit,
        plugins: snapshot_plugins(state),
    }
}

fn snapshot_timestamp(state: &Mutex<PluginCpuState>) -> u64 {
    let fallback = now_millis();
    let Ok(state) = state.lock() else {
        return fallback;
    };
    state.last_timestamp_ms.max(fallback)
}

fn snapshot_plugins(state: &Mutex<PluginCpuState>) -> Vec<PluginCpuEntry> {
    let Ok(state) = state.lock() else {
        return Vec::new();
    };

    let mut plugins = state
        .plugin_rows
        .iter()
        .map(|(plugin_id, row)| PluginCpuEntry {
            plugin_id: plugin_id.clone(),
            daemon_pid: row.daemon_pid,
            action_pids: row.action_pids.clone(),
            cpu_percent: row.cpu_percent,
            cpu_seconds_total: row.cpu_seconds_total,
            history: row.history.iter().cloned().collect(),
        })
        .collect::<Vec<_>>();
    plugins.sort_by(compare_plugin_entries);
    plugins
}

fn compare_plugin_entries(left: &PluginCpuEntry, right: &PluginCpuEntry) -> Ordering {
    right
        .cpu_percent
        .partial_cmp(&left.cpu_percent)
        .unwrap_or(Ordering::Equal)
        .then_with(|| left.plugin_id.cmp(&right.plugin_id))
}
