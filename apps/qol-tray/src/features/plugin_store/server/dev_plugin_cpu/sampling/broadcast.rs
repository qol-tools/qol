use std::sync::{Arc, Mutex};

use super::super::snapshot::PluginCpuEntry;
use super::super::state::{PluginCpuRow, PluginCpuState};

pub(super) fn snapshot_for_broadcast(
    state: &Arc<Mutex<PluginCpuState>>,
) -> Option<(u64, Vec<PluginCpuEntry>)> {
    let Ok(guard) = state.lock() else { return None; };
    if guard.plugin_rows.is_empty() { return None; }
    let timestamp_ms = guard.last_timestamp_ms;
    let plugins = guard.plugin_rows.iter().map(|(id, row)| to_entry(id, row)).collect();
    Some((timestamp_ms, plugins))
}

fn to_entry(plugin_id: &str, row: &PluginCpuRow) -> PluginCpuEntry {
    PluginCpuEntry {
        plugin_id: plugin_id.to_owned(),
        daemon_pid: row.daemon_pid,
        action_pids: row.action_pids.clone(),
        cpu_percent: row.cpu_percent,
        cpu_seconds_total: row.cpu_seconds_total,
        history: row.history.iter().cloned().collect(),
    }
}
