use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use crate::plugins::PluginManager;

use super::super::state::PluginPidSet;

pub(super) fn active_plugin_ids(plugin_pids: &HashMap<String, PluginPidSet>) -> HashSet<String> {
    plugin_pids.keys().cloned().collect()
}

pub(super) fn active_pids(plugin_pids: &HashMap<String, PluginPidSet>) -> HashSet<i32> {
    plugin_pids
        .values()
        .flat_map(|set| set.all_pids.iter().map(|pid| *pid as i32))
        .collect()
}

pub(super) fn current_cpu_by_pid(active_pids: &HashSet<i32>) -> HashMap<i32, u64> {
    let mut current_cpu_by_pid = HashMap::new();
    for pid in active_pids {
        if let Some(cpu_micros) = super::super::platform::process_cpu_micros(*pid) {
            current_cpu_by_pid.insert(*pid, cpu_micros);
        }
    }
    current_cpu_by_pid
}

pub(super) fn collect_plugin_pids(
    plugin_manager: &Arc<Mutex<PluginManager>>,
) -> HashMap<String, PluginPidSet> {
    let daemon_pids = collect_daemon_pids(plugin_manager);
    let action_pids = crate::plugins::action_executor::action_processes_snapshot();
    build_pid_sets(daemon_pids, action_pids)
}

fn collect_daemon_pids(plugin_manager: &Arc<Mutex<PluginManager>>) -> HashMap<String, u32> {
    match plugin_manager.lock() {
        Ok(manager) => manager
            .plugins()
            .filter_map(|plugin| plugin.daemon_pid().map(|pid| (plugin.id.to_string(), pid)))
            .collect(),
        Err(error) => {
            log::error!(
                "Plugin manager lock poisoned for CPU diagnostics: {}",
                error
            );
            HashMap::new()
        }
    }
}

fn build_pid_sets(
    daemon_pids: HashMap<String, u32>,
    action_pids: HashMap<String, Vec<u32>>,
) -> HashMap<String, PluginPidSet> {
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
