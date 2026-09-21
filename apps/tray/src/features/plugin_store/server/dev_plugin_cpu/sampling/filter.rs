use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use super::super::state::{PluginCpuState, PluginPidSet};
use super::now_millis;

pub(super) fn filter_monitored_plugins(
    state: &Arc<Mutex<PluginCpuState>>,
    plugin_pids: &mut HashMap<String, PluginPidSet>,
) -> Option<HashMap<String, PluginPidSet>> {
    let monitored_plugin_ids = monitored_plugin_filter(state);
    let Some(monitored_plugin_ids) = monitored_plugin_ids else {
        return Some(std::mem::take(plugin_pids));
    };
    if monitored_plugin_ids.is_empty() {
        clear_sample_state(state);
        return None;
    }
    plugin_pids.retain(|plugin_id, _| monitored_plugin_ids.contains(plugin_id));
    Some(std::mem::take(plugin_pids))
}

pub(super) fn set_monitored_plugins(state: &Mutex<PluginCpuState>, plugin_ids: Vec<String>) {
    let monitored_plugin_ids = plugin_ids.into_iter().collect::<HashSet<_>>();
    let Ok(mut state) = state.lock() else {
        return;
    };
    state.monitoring_filter_enabled = true;
    state.monitored_plugin_ids = monitored_plugin_ids;
    let monitored = state.monitored_plugin_ids.clone();
    state
        .plugin_rows
        .retain(|plugin_id, _| monitored.contains(plugin_id));
    if !state.plugin_rows.is_empty() {
        return;
    }
    state.pid_cpu_micros.clear();
}

fn monitored_plugin_filter(state: &Arc<Mutex<PluginCpuState>>) -> Option<HashSet<String>> {
    let Ok(guard) = state.lock() else {
        return None;
    };
    if !guard.monitoring_filter_enabled {
        return None;
    }
    Some(guard.monitored_plugin_ids.clone())
}

fn clear_sample_state(state: &Arc<Mutex<PluginCpuState>>) {
    let Ok(mut guard) = state.lock() else {
        return;
    };
    guard.plugin_rows.clear();
    guard.pid_cpu_micros.clear();
    guard.last_sample_at = None;
    guard.last_timestamp_ms = now_millis();
}
