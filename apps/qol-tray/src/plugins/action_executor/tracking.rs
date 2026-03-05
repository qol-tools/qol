use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

static ACTION_PROCESSES: OnceLock<Mutex<HashMap<String, Vec<u32>>>> = OnceLock::new();
static RUNNING_ACTIONS: OnceLock<Mutex<HashMap<String, u32>>> = OnceLock::new();

#[cfg(any(test, feature = "dev"))]
pub(super) fn action_processes_snapshot() -> HashMap<String, Vec<u32>> {
    action_processes()
        .lock()
        .map(|processes| processes.clone())
        .unwrap_or_default()
}

#[cfg(test)]
pub(super) fn running_actions_snapshot() -> HashMap<String, u32> {
    running_actions()
        .lock()
        .map(|actions| actions.clone())
        .unwrap_or_default()
}

#[cfg(test)]
pub(super) fn clear_tracking() {
    clear_action_processes();
    clear_running_actions();
}

pub fn kill_all_plugin_processes() {
    crate::process_utils::reap_children_nonblocking();
    kill_tracked_processes();
    clear_running_actions();
    crate::process_utils::reap_children_nonblocking();
}

pub(super) fn reserve_runtime_spawn(plugin_id: &str, action_id: &str) -> bool {
    let key = action_key(plugin_id, action_id);
    let Ok(mut running) = running_actions().lock() else {
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

pub(super) fn clear_runtime_spawn_reservation(plugin_id: &str, action_id: &str) {
    let key = action_key(plugin_id, action_id);
    let Ok(mut running) = running_actions().lock() else {
        return;
    };
    if running.get(&key).copied() == Some(0) {
        running.remove(&key);
    }
}

pub(super) fn track_action_process(plugin_id: &str, action_id: &str, pid: u32) {
    push_action_process(plugin_id, pid);
    remember_running_action(plugin_id, action_id, pid);
    #[cfg(unix)]
    crate::desktop_state::add_ignore_pid(pid);
}

pub(super) fn untrack_action_process(plugin_id: &str, action_id: &str, pid: u32) {
    remove_action_process(plugin_id, pid);
    forget_running_action(plugin_id, action_id, pid);
    #[cfg(unix)]
    crate::desktop_state::remove_ignore_pid(pid);
}

fn action_processes() -> &'static Mutex<HashMap<String, Vec<u32>>> {
    ACTION_PROCESSES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn running_actions() -> &'static Mutex<HashMap<String, u32>> {
    RUNNING_ACTIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn action_key(plugin_id: &str, action_id: &str) -> String {
    format!("{plugin_id}::{action_id}")
}

#[cfg(test)]
fn clear_action_processes() {
    let Ok(mut processes) = action_processes().lock() else {
        return;
    };
    processes.clear();
}

fn clear_running_actions() {
    let Ok(mut running) = running_actions().lock() else {
        return;
    };
    running.clear();
}

fn kill_tracked_processes() {
    let Ok(mut processes) = action_processes().lock() else {
        log::error!("Failed to lock action processes");
        return;
    };

    for (plugin_id, pids) in processes.drain() {
        for pid in pids {
            kill_process(pid, &plugin_id);
        }
    }
}

fn kill_process(pid: u32, plugin_id: &str) {
    let pid = pid as i32;
    if crate::process_utils::is_pid_alive(pid) {
        log::info!("Killing action process {} for plugin {}", pid, plugin_id);
        crate::process_utils::terminate_pid(pid, std::time::Duration::from_millis(100));
    }
}

fn push_action_process(plugin_id: &str, pid: u32) {
    let Ok(mut processes) = action_processes().lock() else {
        return;
    };
    processes
        .entry(plugin_id.to_string())
        .or_default()
        .push(pid);
}

fn remember_running_action(plugin_id: &str, action_id: &str, pid: u32) {
    let Ok(mut running) = running_actions().lock() else {
        return;
    };
    running.insert(action_key(plugin_id, action_id), pid);
}

fn remove_action_process(plugin_id: &str, pid: u32) {
    let Ok(mut processes) = action_processes().lock() else {
        return;
    };
    let remove_entry = process_entry_empty(plugin_id, pid, &mut processes);
    if remove_entry {
        processes.remove(plugin_id);
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

fn forget_running_action(plugin_id: &str, action_id: &str, pid: u32) {
    let key = action_key(plugin_id, action_id);
    let Ok(mut running) = running_actions().lock() else {
        return;
    };
    if running.get(&key).copied() == Some(pid) {
        running.remove(&key);
    }
}
