use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Default)]
pub(crate) struct ProcessTracker {
    action_processes: Mutex<HashMap<String, Vec<u32>>>,
    running_actions: Mutex<HashMap<String, u32>>,
}

impl ProcessTracker {
    #[cfg(any(test, feature = "dev"))]
    pub(crate) fn action_processes_snapshot(&self) -> HashMap<String, Vec<u32>> {
        self.action_processes
            .lock()
            .map(|processes| processes.clone())
            .unwrap_or_default()
    }

    #[cfg(test)]
    pub(super) fn running_actions_snapshot(&self) -> HashMap<String, u32> {
        self.running_actions
            .lock()
            .map(|actions| actions.clone())
            .unwrap_or_default()
    }

    pub(crate) fn kill_all_plugin_processes(&self) {
        self.kill_tracked_processes();
        self.clear_running_actions();
    }

    pub(crate) fn kill_plugin_processes(&self, plugin_id: &str) {
        self.kill_tracked_processes_for_plugin(plugin_id);
        self.clear_running_actions_for_plugin(plugin_id);
    }

    pub(super) fn reserve_runtime_spawn(&self, plugin_id: &str, action_id: &str) -> bool {
        let key = action_key(plugin_id, action_id);
        let Ok(mut running) = self.running_actions.lock() else {
            return false;
        };

        let Some(existing_pid) = running.get(&key).copied() else {
            running.insert(key, 0);
            return true;
        };
        if existing_pid == 0 {
            log::info!(
                "Plugin action spawn already in progress, skipping: {}::{}",
                plugin_id,
                action_id
            );
            return false;
        }
        if crate::process_utils::is_pid_alive(existing_pid as i32) {
            log::info!(
                "Plugin action already running, skipping spawn: {}::{} (pid: {})",
                plugin_id,
                action_id,
                existing_pid
            );
            return false;
        }

        running.insert(key, 0);
        true
    }

    pub(super) fn clear_runtime_spawn_reservation(&self, plugin_id: &str, action_id: &str) {
        let key = action_key(plugin_id, action_id);
        let Ok(mut running) = self.running_actions.lock() else {
            return;
        };
        if running.get(&key).copied() == Some(0) {
            running.remove(&key);
        }
    }

    pub(super) fn track_action_process(&self, plugin_id: &str, action_id: &str, pid: u32) {
        self.push_action_process(plugin_id, pid);
        self.remember_running_action(plugin_id, action_id, pid);
        super::platform::track_desktop_state_pid(pid);
    }

    pub(super) fn track_unreserved_action_process(&self, plugin_id: &str, pid: u32) {
        self.push_action_process(plugin_id, pid);
        super::platform::track_desktop_state_pid(pid);
    }

    pub(super) fn untrack_action_process(&self, plugin_id: &str, action_id: &str, pid: u32) {
        self.remove_action_process(plugin_id, pid);
        self.forget_running_action(plugin_id, action_id, pid);
        super::platform::untrack_desktop_state_pid(pid);
    }

    fn clear_running_actions(&self) {
        let Ok(mut running) = self.running_actions.lock() else {
            return;
        };
        running.clear();
    }

    fn clear_running_actions_for_plugin(&self, plugin_id: &str) {
        let Ok(mut running) = self.running_actions.lock() else {
            return;
        };
        let prefix = format!("{plugin_id}::");
        running.retain(|key, _| !key.starts_with(&prefix));
    }

    fn kill_tracked_processes(&self) {
        let Ok(mut processes) = self.action_processes.lock() else {
            log::error!("Failed to lock action processes");
            return;
        };

        for (plugin_id, pids) in processes.drain() {
            for pid in pids {
                kill_process(pid, &plugin_id);
            }
        }
    }

    fn kill_tracked_processes_for_plugin(&self, plugin_id: &str) {
        let Ok(mut processes) = self.action_processes.lock() else {
            log::error!("Failed to lock action processes");
            return;
        };
        let Some(pids) = processes.remove(plugin_id) else {
            return;
        };
        for pid in pids {
            kill_process(pid, plugin_id);
        }
    }

    fn push_action_process(&self, plugin_id: &str, pid: u32) {
        let Ok(mut processes) = self.action_processes.lock() else {
            return;
        };
        processes
            .entry(plugin_id.to_string())
            .or_default()
            .push(pid);
    }

    fn remember_running_action(&self, plugin_id: &str, action_id: &str, pid: u32) {
        let Ok(mut running) = self.running_actions.lock() else {
            return;
        };
        running.insert(action_key(plugin_id, action_id), pid);
    }

    fn remove_action_process(&self, plugin_id: &str, pid: u32) {
        let Ok(mut processes) = self.action_processes.lock() else {
            return;
        };
        let remove_entry = process_entry_empty(plugin_id, pid, &mut processes);
        if remove_entry {
            processes.remove(plugin_id);
        }
    }

    fn forget_running_action(&self, plugin_id: &str, action_id: &str, pid: u32) {
        let key = action_key(plugin_id, action_id);
        let Ok(mut running) = self.running_actions.lock() else {
            return;
        };
        if running.get(&key).copied() == Some(pid) {
            running.remove(&key);
        }
    }
}

fn action_key(plugin_id: &str, action_id: &str) -> String {
    format!("{plugin_id}::{action_id}")
}

fn kill_process(pid: u32, plugin_id: &str) {
    let pid = pid as i32;
    if crate::process_utils::is_pid_alive(pid) {
        log::info!("Killing action process {} for plugin {}", pid, plugin_id);
        crate::process_utils::terminate_pid(pid, std::time::Duration::from_millis(100));
    }
}

fn process_entry_empty(
    plugin_id: &str,
    pid: u32,
    processes: &mut HashMap<String, Vec<u32>>,
) -> bool {
    let Some(pids) = processes.get_mut(plugin_id) else {
        return false;
    };
    pids.retain(|tracked| *tracked != pid);
    pids.is_empty()
}
