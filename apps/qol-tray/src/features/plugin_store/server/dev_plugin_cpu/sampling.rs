#![cfg(feature = "dev")]

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use crate::daemon::{DaemonEvent, EventBus};
use crate::plugins::PluginManager;

use super::platform;
use super::snapshot::{PluginCpuEntry, PluginCpuPoint};
use super::state::{PluginCpuRow, PluginCpuState, PluginPidSet};

pub(super) fn sample_once(
    state: &Arc<Mutex<PluginCpuState>>,
    plugin_manager: &Arc<Mutex<PluginManager>>,
    history_limit: usize,
) {
    let cpu_percent_window_samples = platform::cpu_percent_window_samples().max(1);
    let mut plugin_pids = collect_plugin_pids(plugin_manager);
    let Some(plugin_pids) = filter_monitored_plugins(state, &mut plugin_pids) else {
        return;
    };
    let active_plugins: HashSet<String> = plugin_pids.keys().cloned().collect();
    let active_pids = active_pids(&plugin_pids);
    let current_cpu_by_pid = current_cpu_by_pid(&active_pids);
    let now = Instant::now();
    let timestamp_ms = now_millis();

    let Ok(mut guard) = state.lock() else {
        return;
    };

    let elapsed = elapsed_seconds(&guard, now);
    guard.last_sample_at = Some(now);
    guard.last_timestamp_ms = timestamp_ms;
    retain_active(&mut guard, &active_pids, &active_plugins);

    for (plugin_id, pid_set) in plugin_pids {
        sample_plugin_row(
            &mut guard,
            plugin_id,
            pid_set,
            &current_cpu_by_pid,
            elapsed,
            timestamp_ms,
            cpu_percent_window_samples,
            history_limit,
        );
    }
}

pub(super) fn broadcast_snapshot(state: &Arc<Mutex<PluginCpuState>>, events: &Arc<EventBus>) {
    let Some((timestamp_ms, plugins)) = snapshot_for_broadcast(state) else {
        return;
    };
    events.send(DaemonEvent::PluginCpuSnapshot {
        timestamp_ms,
        plugins,
    });
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

pub(super) fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn filter_monitored_plugins(
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

fn monitored_plugin_filter(state: &Arc<Mutex<PluginCpuState>>) -> Option<HashSet<String>> {
    let Ok(guard) = state.lock() else {
        return None;
    };
    if !guard.monitoring_filter_enabled {
        return None;
    }
    Some(guard.monitored_plugin_ids.clone())
}

fn active_pids(plugin_pids: &HashMap<String, PluginPidSet>) -> HashSet<i32> {
    plugin_pids
        .values()
        .flat_map(|set| set.all_pids.iter().map(|pid| *pid as i32))
        .collect()
}

fn current_cpu_by_pid(active_pids: &HashSet<i32>) -> HashMap<i32, u64> {
    let mut current_cpu_by_pid = HashMap::new();
    for pid in active_pids {
        if let Some(cpu_micros) = platform::process_cpu_micros(*pid) {
            current_cpu_by_pid.insert(*pid, cpu_micros);
        }
    }
    current_cpu_by_pid
}

fn elapsed_seconds(state: &PluginCpuState, now: Instant) -> f64 {
    state
        .last_sample_at
        .map(|last| now.duration_since(last).as_secs_f64())
        .unwrap_or(0.0)
}

fn retain_active(
    state: &mut PluginCpuState,
    active_pids: &HashSet<i32>,
    active_plugins: &HashSet<String>,
) {
    state
        .pid_cpu_micros
        .retain(|pid, _| active_pids.contains(pid));
    state
        .plugin_rows
        .retain(|plugin_id, _| active_plugins.contains(plugin_id));
}

fn sample_plugin_row(
    state: &mut PluginCpuState,
    plugin_id: String,
    pid_set: PluginPidSet,
    current_cpu_by_pid: &HashMap<i32, u64>,
    elapsed: f64,
    timestamp_ms: u64,
    cpu_percent_window_samples: usize,
    history_limit: usize,
) {
    let (cpu_percent, cpu_total_micros) = sample_plugin_cpu(
        &mut state.pid_cpu_micros,
        &pid_set,
        current_cpu_by_pid,
        elapsed,
    );
    let row = state
        .plugin_rows
        .entry(plugin_id)
        .or_insert_with(PluginCpuRow::default);
    row.daemon_pid = pid_set.daemon_pid;
    row.action_pids = pid_set.action_pids;
    update_smoothed_cpu_percent(row, cpu_percent, cpu_percent_window_samples);
    row.cpu_seconds_total = cpu_total_micros as f64 / 1_000_000.0;
    row.history.push_back(PluginCpuPoint {
        timestamp_ms,
        cpu_percent: row.cpu_percent,
    });
    while row.history.len() > history_limit {
        row.history.pop_front();
    }
}

fn sample_plugin_cpu(
    pid_cpu_micros: &mut HashMap<i32, u64>,
    pid_set: &PluginPidSet,
    current_cpu_by_pid: &HashMap<i32, u64>,
    elapsed: f64,
) -> (f64, u128) {
    let mut cpu_percent = 0.0;
    let mut cpu_total_micros: u128 = 0;

    for pid in &pid_set.all_pids {
        let pid_i32 = *pid as i32;
        let Some(current_cpu_micros) = current_cpu_by_pid.get(&pid_i32).copied() else {
            continue;
        };
        cpu_total_micros += current_cpu_micros as u128;
        let previous_cpu_micros = pid_cpu_micros.insert(pid_i32, current_cpu_micros);
        let Some(previous_cpu_micros) = previous_cpu_micros else {
            continue;
        };
        if elapsed <= 0.0 {
            continue;
        }
        let delta_micros = current_cpu_micros.saturating_sub(previous_cpu_micros) as f64;
        cpu_percent += (delta_micros / 1_000_000.0) / elapsed * 100.0;
    }

    (cpu_percent, cpu_total_micros)
}

fn snapshot_for_broadcast(
    state: &Arc<Mutex<PluginCpuState>>,
) -> Option<(u64, Vec<PluginCpuEntry>)> {
    let Ok(guard) = state.lock() else {
        return None;
    };
    if guard.plugin_rows.is_empty() {
        return None;
    }
    let timestamp_ms = guard.last_timestamp_ms;
    let plugins = guard
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
        .collect();
    Some((timestamp_ms, plugins))
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

fn update_smoothed_cpu_percent(
    row: &mut PluginCpuRow,
    raw_cpu_percent: f64,
    window_samples: usize,
) {
    row.cpu_percent_samples
        .push_back(round_two_decimals(raw_cpu_percent));
    while row.cpu_percent_samples.len() > window_samples {
        row.cpu_percent_samples.pop_front();
    }
    if row.cpu_percent_samples.is_empty() {
        row.cpu_percent = 0.0;
        return;
    }
    let averaged_cpu_percent =
        row.cpu_percent_samples.iter().sum::<f64>() / row.cpu_percent_samples.len() as f64;
    row.cpu_percent = round_two_decimals(averaged_cpu_percent);
}

fn round_two_decimals(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

fn collect_plugin_pids(
    plugin_manager: &Arc<Mutex<PluginManager>>,
) -> HashMap<String, PluginPidSet> {
    let daemon_pids = match plugin_manager.lock() {
        Ok(manager) => manager
            .plugins()
            .filter_map(|plugin| plugin.daemon_pid().map(|pid| (plugin.id.clone(), pid)))
            .collect::<HashMap<String, u32>>(),
        Err(error) => {
            log::error!(
                "Plugin manager lock poisoned for CPU diagnostics: {}",
                error
            );
            HashMap::new()
        }
    };

    let action_pids = crate::plugins::action_executor::action_processes_snapshot();
    let mut plugin_ids: HashSet<String> = daemon_pids.keys().cloned().collect();
    plugin_ids.extend(action_pids.keys().cloned());

    let mut result = HashMap::new();
    for plugin_id in plugin_ids {
        let daemon_pid = daemon_pids.get(&plugin_id).copied();
        let mut actions = action_pids.get(&plugin_id).cloned().unwrap_or_default();
        actions.sort_unstable();
        actions.dedup();

        let mut all_pids = actions.clone();
        if let Some(pid) = daemon_pid {
            all_pids.push(pid);
        }
        all_pids.sort_unstable();
        all_pids.dedup();

        result.insert(
            plugin_id,
            PluginPidSet {
                daemon_pid,
                action_pids: actions,
                all_pids,
            },
        );
    }
    result
}
