use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use qol_window_state::{Reopen, WindowStateStore};
use serde::{Deserialize, Serialize};

use crate::plugins::action_executor::{self, ActionExecutionError};
use crate::plugins::PluginManager;

const REOPEN_FILE: &str = ".reopen-windows.json";
const REOPEN_FRESH_FOR: Duration = Duration::from_secs(120);
const DAEMON_READY_TIMEOUT: Duration = Duration::from_secs(30);
const DAEMON_READY_POLL: Duration = Duration::from_millis(250);
const SETTINGS_READY_TIMEOUT: Duration = Duration::from_secs(30);
const SETTINGS_FALLBACK_PAGE: &str = "__core-plugins";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct ReopenList {
    written_at_ms: u64,
    windows: Vec<Reopen>,
}

pub(crate) fn capture_before_restart() {
    let (Some(store), Ok(dir)) = (
        WindowStateStore::shared(),
        crate::paths::shared_config_dir(),
    ) else {
        return;
    };
    let windows = open_windows(&store, qol_process::is_pid_alive);
    match write_list(&dir, &windows, now_ms()) {
        Ok(()) => log::info!(
            "[window-state] {} windows to reopen after restart",
            windows.len()
        ),
        Err(error) => log::warn!("[window-state] failed to record windows to reopen: {error}"),
    }
}

pub(crate) fn discard_reopen_list() {
    let Ok(dir) = crate::paths::shared_config_dir() else {
        return;
    };
    if let Err(error) = std::fs::remove_file(list_path(&dir)) {
        if error.kind() != io::ErrorKind::NotFound {
            log::warn!("[window-state] failed to discard the reopen list: {error}");
        }
    }
}

pub fn reopen_after_restart(plugin_manager: Arc<Mutex<PluginManager>>) {
    if crate::dev_generation::is_shadow() {
        return;
    }
    let Ok(dir) = crate::paths::shared_config_dir() else {
        return;
    };
    let windows = take_list(&dir, now_ms());
    if windows.is_empty() {
        return;
    }
    std::thread::spawn(move || {
        for window in windows {
            reopen(&plugin_manager, window);
        }
    });
}

fn reopen(plugin_manager: &Arc<Mutex<PluginManager>>, window: Reopen) {
    match window {
        Reopen::PluginAction { plugin, action } => {
            run_when_ready(plugin_manager, &plugin, &action);
        }
        Reopen::Settings { page } => {
            let _ = crate::settings_surface::wait_until_ready(SETTINGS_READY_TIMEOUT);
            let page = page.unwrap_or_else(|| SETTINGS_FALLBACK_PAGE.to_string());
            if let Err(error) = crate::settings_surface::request(&page) {
                log::warn!("[window-state] failed to reopen settings on {page}: {error:#}");
            }
        }
    }
}

fn run_when_ready(plugin_manager: &Arc<Mutex<PluginManager>>, plugin: &str, action: &str) {
    let deadline = std::time::Instant::now() + DAEMON_READY_TIMEOUT;
    loop {
        match action_executor::try_execute_action(plugin_manager, plugin, action) {
            Ok(()) => return,
            Err(ActionExecutionError::DaemonNotReady { .. })
                if std::time::Instant::now() < deadline =>
            {
                std::thread::sleep(DAEMON_READY_POLL);
            }
            Err(error) => {
                log::warn!("[window-state] failed to reopen {plugin}::{action}: {error}");
                return;
            }
        }
    }
}

fn open_windows(store: &WindowStateStore, owner_alive: impl Fn(u32) -> bool) -> Vec<Reopen> {
    let mut windows = Vec::new();
    for state in store.list() {
        let Some(reopen) = state.reopen else {
            continue;
        };
        if state.open && owner_alive(state.owner_pid) && !windows.contains(&reopen) {
            windows.push(reopen);
        }
    }
    windows
}

fn list_path(dir: &Path) -> PathBuf {
    dir.join(REOPEN_FILE)
}

fn write_list(dir: &Path, windows: &[Reopen], now_ms: u64) -> io::Result<()> {
    let list = ReopenList {
        written_at_ms: now_ms,
        windows: windows.to_vec(),
    };
    let json = serde_json::to_vec(&list).map_err(io::Error::other)?;
    qol_fs::atomic_write_durable(&list_path(dir), &json)
}

fn take_list(dir: &Path, now_ms: u64) -> Vec<Reopen> {
    let path = list_path(dir);
    let Ok(bytes) = std::fs::read(&path) else {
        return Vec::new();
    };
    if let Err(error) = std::fs::remove_file(&path) {
        log::warn!("[window-state] failed to delete the reopen list: {error}");
        return Vec::new();
    }
    let Ok(list) = serde_json::from_slice::<ReopenList>(&bytes) else {
        return Vec::new();
    };
    let age = Duration::from_millis(now_ms.saturating_sub(list.written_at_ms));
    if age > REOPEN_FRESH_FOR {
        return Vec::new();
    }
    list.windows
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests;
