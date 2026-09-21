use std::collections::{HashMap, HashSet};
use std::time::Instant;

pub(super) struct RowConfig {
    pub(super) cpu_percent_window_samples: usize,
    pub(super) history_limit: usize,
}

use super::super::snapshot::PluginCpuPoint;
use super::super::state::{PluginCpuRow, PluginCpuState, PluginPidSet};

pub(super) fn elapsed_seconds(state: &PluginCpuState, now: Instant) -> f64 {
    state
        .last_sample_at
        .map(|last| now.duration_since(last).as_secs_f64())
        .unwrap_or(0.0)
}

pub(super) fn retain_active(
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

pub(super) fn sample_plugin_row(
    state: &mut PluginCpuState,
    plugin_id: String,
    pid_set: PluginPidSet,
    current_cpu_by_pid: &HashMap<i32, u64>,
    elapsed: f64,
    timestamp_ms: u64,
    config: &RowConfig,
) {
    let cpu_percent_window_samples = config.cpu_percent_window_samples;
    let history_limit = config.history_limit;
    let (cpu_percent, cpu_total_micros) = sample_plugin_cpu(
        &mut state.pid_cpu_micros,
        &pid_set,
        current_cpu_by_pid,
        elapsed,
    );
    let row = state.plugin_rows.entry(plugin_id).or_default();
    row.daemon_pid = pid_set.daemon_pid;
    row.action_pids = pid_set.action_pids;
    update_smoothed_cpu_percent(row, cpu_percent, cpu_percent_window_samples);
    row.cpu_seconds_total = cpu_total_micros as f64 / 1_000_000.0;
    update_row_history(row, timestamp_ms, history_limit);
}

fn sample_pid_cpu_percent(
    pid_cpu_micros: &mut HashMap<i32, u64>,
    pid_i32: i32,
    current_cpu_micros: u64,
    elapsed: f64,
) -> f64 {
    let previous = pid_cpu_micros.insert(pid_i32, current_cpu_micros);
    let Some(previous) = previous else {
        return 0.0;
    };
    if elapsed <= 0.0 {
        return 0.0;
    }
    let delta_micros = current_cpu_micros.saturating_sub(previous) as f64;
    (delta_micros / 1_000_000.0) / elapsed * 100.0
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
        cpu_percent += sample_pid_cpu_percent(pid_cpu_micros, pid_i32, current_cpu_micros, elapsed);
    }
    (cpu_percent, cpu_total_micros)
}

fn update_row_history(row: &mut PluginCpuRow, timestamp_ms: u64, history_limit: usize) {
    row.history.push_back(PluginCpuPoint {
        timestamp_ms,
        cpu_percent: row.cpu_percent,
    });
    while row.history.len() > history_limit {
        row.history.pop_front();
    }
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
