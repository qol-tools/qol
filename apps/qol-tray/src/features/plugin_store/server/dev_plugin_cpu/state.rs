#![cfg(feature = "dev")]

use std::collections::{HashMap, HashSet, VecDeque};
use std::time::Instant;

use super::snapshot::PluginCpuPoint;

#[derive(Default)]
pub(super) struct PluginCpuRow {
    pub(super) daemon_pid: Option<u32>,
    pub(super) action_pids: Vec<u32>,
    pub(super) cpu_percent: f64,
    pub(super) cpu_seconds_total: f64,
    pub(super) cpu_percent_samples: VecDeque<f64>,
    pub(super) history: VecDeque<PluginCpuPoint>,
}

#[derive(Default)]
pub(super) struct PluginCpuState {
    pub(super) last_sample_at: Option<Instant>,
    pub(super) last_timestamp_ms: u64,
    pub(super) pid_cpu_micros: HashMap<i32, u64>,
    pub(super) plugin_rows: HashMap<String, PluginCpuRow>,
    pub(super) monitored_plugin_ids: HashSet<String>,
    pub(super) monitoring_filter_enabled: bool,
}

#[derive(Default)]
pub(super) struct PluginPidSet {
    pub(super) daemon_pid: Option<u32>,
    pub(super) action_pids: Vec<u32>,
    pub(super) all_pids: Vec<u32>,
}
