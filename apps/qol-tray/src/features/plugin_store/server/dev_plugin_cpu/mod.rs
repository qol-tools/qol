#![cfg(feature = "dev")]

mod platform;

use crate::plugins::PluginManager;
use serde::Serialize;
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const SAMPLE_INTERVAL: Duration = Duration::from_secs(1);
const HISTORY_LIMIT: usize = 60;

#[derive(Debug, Clone, Serialize, Default)]
pub(super) struct PluginCpuPoint {
    pub(super) timestamp_ms: u64,
    pub(super) cpu_percent: f64,
}

#[derive(Debug, Clone, Serialize, Default)]
pub(super) struct PluginCpuEntry {
    pub(super) plugin_id: String,
    pub(super) daemon_pid: Option<u32>,
    pub(super) action_pids: Vec<u32>,
    pub(super) cpu_percent: f64,
    pub(super) cpu_seconds_total: f64,
    pub(super) history: Vec<PluginCpuPoint>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub(super) struct PluginCpuResponse {
    pub(super) timestamp_ms: u64,
    pub(super) sample_interval_ms: u64,
    pub(super) history_limit: usize,
    pub(super) plugins: Vec<PluginCpuEntry>,
}

#[derive(Default)]
struct PluginCpuRow {
    daemon_pid: Option<u32>,
    action_pids: Vec<u32>,
    cpu_percent: f64,
    cpu_seconds_total: f64,
    cpu_percent_samples: VecDeque<f64>,
    history: VecDeque<PluginCpuPoint>,
}

#[derive(Default)]
struct PluginCpuState {
    last_sample_at: Option<Instant>,
    last_timestamp_ms: u64,
    pid_cpu_micros: HashMap<i32, u64>,
    plugin_rows: HashMap<String, PluginCpuRow>,
}

#[derive(Default)]
struct PluginPidSet {
    daemon_pid: Option<u32>,
    action_pids: Vec<u32>,
    all_pids: Vec<u32>,
}

pub(super) struct DevPluginCpuService {
    state: Arc<Mutex<PluginCpuState>>,
}

impl DevPluginCpuService {
    pub(super) fn start(plugin_manager: Arc<Mutex<PluginManager>>) -> Arc<Self> {
        let service = Arc::new(Self {
            state: Arc::new(Mutex::new(PluginCpuState::default())),
        });
        let state = service.state.clone();
        tokio::spawn(async move {
            loop {
                sample_once(&state, &plugin_manager);
                tokio::time::sleep(SAMPLE_INTERVAL).await;
            }
        });
        service
    }

    pub(super) fn snapshot(&self) -> PluginCpuResponse {
        let mut timestamp_ms = now_millis();
        let mut plugins = Vec::new();
        if let Ok(state) = self.state.lock() {
            timestamp_ms = state.last_timestamp_ms.max(timestamp_ms);
            plugins = state
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
        }
        plugins.sort_by(|left, right| {
            right
                .cpu_percent
                .partial_cmp(&left.cpu_percent)
                .unwrap_or(Ordering::Equal)
                .then_with(|| left.plugin_id.cmp(&right.plugin_id))
        });
        PluginCpuResponse {
            timestamp_ms,
            sample_interval_ms: SAMPLE_INTERVAL.as_millis() as u64,
            history_limit: HISTORY_LIMIT,
            plugins,
        }
    }
}

fn sample_once(state: &Arc<Mutex<PluginCpuState>>, plugin_manager: &Arc<Mutex<PluginManager>>) {
    let cpu_percent_window_samples = platform::cpu_percent_window_samples().max(1);
    let plugin_pids = collect_plugin_pids(plugin_manager);
    let active_plugins: HashSet<String> = plugin_pids.keys().cloned().collect();
    let active_pids: HashSet<i32> = plugin_pids
        .values()
        .flat_map(|set| set.all_pids.iter().map(|pid| *pid as i32))
        .collect();

    let mut current_cpu_by_pid = HashMap::new();
    for pid in &active_pids {
        if let Some(cpu_micros) = platform::process_cpu_micros(*pid) {
            current_cpu_by_pid.insert(*pid, cpu_micros);
        }
    }

    let now = Instant::now();
    let timestamp_ms = now_millis();

    let Ok(mut guard) = state.lock() else {
        return;
    };

    let elapsed = guard
        .last_sample_at
        .map(|last| now.duration_since(last).as_secs_f64())
        .unwrap_or(0.0);
    guard.last_sample_at = Some(now);
    guard.last_timestamp_ms = timestamp_ms;

    guard
        .pid_cpu_micros
        .retain(|pid, _| active_pids.contains(pid));
    guard
        .plugin_rows
        .retain(|plugin_id, _| active_plugins.contains(plugin_id));

    for (plugin_id, pid_set) in plugin_pids {
        let mut cpu_percent = 0.0;
        let mut cpu_total_micros: u128 = 0;

        for pid in &pid_set.all_pids {
            let pid_i32 = *pid as i32;
            let Some(current_cpu_micros) = current_cpu_by_pid.get(&pid_i32).copied() else {
                continue;
            };
            cpu_total_micros += current_cpu_micros as u128;
            let previous_cpu_micros = guard.pid_cpu_micros.insert(pid_i32, current_cpu_micros);
            let Some(previous_cpu_micros) = previous_cpu_micros else {
                continue;
            };
            if elapsed <= 0.0 {
                continue;
            }
            let delta_micros = current_cpu_micros.saturating_sub(previous_cpu_micros) as f64;
            cpu_percent += (delta_micros / 1_000_000.0) / elapsed * 100.0;
        }

        let row = guard
            .plugin_rows
            .entry(plugin_id.clone())
            .or_insert_with(PluginCpuRow::default);
        row.daemon_pid = pid_set.daemon_pid;
        row.action_pids = pid_set.action_pids;
        update_smoothed_cpu_percent(row, cpu_percent, cpu_percent_window_samples);
        row.cpu_seconds_total = cpu_total_micros as f64 / 1_000_000.0;
        row.history.push_back(PluginCpuPoint {
            timestamp_ms,
            cpu_percent: row.cpu_percent,
        });
        while row.history.len() > HISTORY_LIMIT {
            row.history.pop_front();
        }
    }
}

fn update_smoothed_cpu_percent(
    row: &mut PluginCpuRow,
    raw_cpu_percent: f64,
    window_samples: usize,
) {
    row.cpu_percent_samples.push_back(round_two_decimals(raw_cpu_percent));
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

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}
