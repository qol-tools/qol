use super::{loading, PluginManager};
use crate::plugins::{action_executor::kill_all_plugin_processes, Plugin};
use anyhow::Result;
use sha2::{Digest, Sha256};

pub(super) fn hash_active_plugin_state() -> String {
    let mut hasher = Sha256::new();
    hash_lock_file(&mut hasher);
    hash_plugin_configs(&mut hasher);
    format!("{:x}", hasher.finalize())
}

fn hash_lock_file(hasher: &mut Sha256) {
    let Some(bytes) = crate::paths::profile_plugins_lock_path()
        .ok()
        .and_then(|p| std::fs::read(&p).ok())
    else {
        return;
    };
    hasher.update(b"lock:");
    hasher.update(&bytes);
}

fn hash_plugin_configs(hasher: &mut Sha256) {
    let Ok(dir) = crate::paths::profile_plugin_configs_dir() else {
        return;
    };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return;
    };
    let mut paths: Vec<_> = entries.filter_map(|e| e.ok()).map(|e| e.path()).collect();
    paths.sort();
    for path in paths {
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        hasher.update(b"cfg:");
        hasher.update(path.to_string_lossy().as_bytes());
        hasher.update(b":");
        hasher.update(&bytes);
    }
}

pub(super) fn reload_plugins(manager: &mut PluginManager) -> Result<()> {
    log::info!("Reloading all plugins...");
    stop_all_plugins(manager);
    loading::load_plugins(manager)?;
    manager.autostart_daemons();
    Ok(())
}

pub(super) fn shutdown(manager: &mut PluginManager) {
    log::info!("Shutting down plugins...");
    stop_all_plugins(manager);
}

pub(super) fn restart_running_plugin_daemon(
    manager: &mut PluginManager,
    plugin_id: &str,
) -> Result<()> {
    let plugin = plugin_mut(manager, plugin_id)?;
    if plugin.daemon_pid().is_none() {
        return Ok(());
    }

    plugin.stop_daemon()?;
    plugin.start_daemon()?;
    sync_ignore_pids(manager);
    Ok(())
}

pub(super) fn ensure_plugin_daemon_running(
    manager: &mut PluginManager,
    plugin_id: &str,
) -> Result<()> {
    start_plugin_daemon_if_needed(manager, plugin_id)?;
    sync_ignore_pids(manager);
    Ok(())
}

pub(super) fn reap_exited_daemons(manager: &mut PluginManager) {
    for plugin in manager.plugins.values_mut() {
        plugin.reap_daemon_if_exited();
    }
}

pub(super) fn sync_ignore_pids(manager: &PluginManager) {
    for plugin in manager.plugins.values() {
        let Some(pid) = plugin.daemon_pid() else {
            continue;
        };
        log::info!("Ignoring daemon pid {} for plugin {}", pid, plugin.id);
        #[cfg(unix)]
        crate::desktop_state::add_ignore_pid(pid);
    }
}

fn stop_all_plugins(manager: &mut PluginManager) {
    kill_all_plugin_processes();
    stop_plugin_daemons(manager);
    manager.plugins.clear();
    super::super::daemon_tracker::registry::clear_all(&crate::paths::runtime_pids_dir());
    super::super::daemon_tracker::kill_orphan_daemons();
}

fn stop_plugin_daemons(manager: &mut PluginManager) {
    for plugin in manager.plugins.values_mut() {
        stop_plugin_daemon(plugin);
    }
}

fn stop_plugin_daemon(plugin: &mut Plugin) {
    if let Err(error) = plugin.stop_daemon() {
        log::error!("Failed to stop daemon for plugin {}: {}", plugin.id, error);
    }
}

fn start_plugin_daemon_if_needed(manager: &mut PluginManager, plugin_id: &str) -> Result<()> {
    if crate::dev_generation::daemon_autostart_held() {
        return Ok(());
    }
    let plugin = plugin_mut(manager, plugin_id)?;
    if !plugin
        .manifest
        .daemon
        .as_ref()
        .is_some_and(|daemon| daemon.enabled)
    {
        return Ok(());
    }
    plugin.reap_daemon_if_exited();
    if plugin.daemon_pid().is_some() {
        return Ok(());
    }
    if crate::dev_generation::is_rolling_restart()
        && super::super::daemon_lifecycle::existing_daemon_socket_ready(plugin)
    {
        return Ok(());
    }

    plugin.start_daemon()
}

fn plugin_mut<'a>(manager: &'a mut PluginManager, plugin_id: &str) -> Result<&'a mut Plugin> {
    manager
        .plugins
        .get_mut(plugin_id)
        .ok_or_else(|| anyhow::anyhow!("plugin not found: {}", plugin_id))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::plugins::manifest::PluginManifest;
    use crate::plugins::PluginId;
    use std::time::{Duration, Instant};

    const PLUGIN_ID: &str = "plugin-foo";

    fn manager_with_running_daemon(root: &std::path::Path) -> PluginManager {
        use std::os::unix::fs::PermissionsExt;
        let plugin_dir = root.join(PLUGIN_ID);
        std::fs::create_dir_all(&plugin_dir).unwrap();
        let script = plugin_dir.join("daemon.sh");
        std::fs::write(&script, "#!/bin/sh\nexec sleep 30\n").unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();

        let manifest: PluginManifest = toml::from_str(
            r#"
[plugin]
id = "plugin-foo"
name = "Foo"
description = ""
version = "1.0.0"

[menu]
label = "Foo"
items = []

[daemon]
enabled = true
command = "daemon.sh"
"#,
        )
        .unwrap();

        let mut manager = PluginManager::new();
        manager.plugins.insert(
            PluginId::new(PLUGIN_ID),
            Plugin::new(PluginId::new(PLUGIN_ID), manifest, plugin_dir),
        );
        manager.ensure_plugin_daemon_running(PLUGIN_ID).unwrap();
        manager
    }

    #[test]
    fn ensure_plugin_daemon_running_respawns_daemon_after_unexpected_exit() {
        let root = tempfile::TempDir::new().unwrap();
        let _guard = crate::paths::push_test_path_root(root.path());
        let mut manager = manager_with_running_daemon(root.path());
        let old_pid = manager
            .get(PLUGIN_ID)
            .unwrap()
            .daemon_pid()
            .expect("daemon spawned");

        unsafe {
            libc::kill(old_pid as i32, libc::SIGKILL);
        }

        let deadline = Instant::now() + Duration::from_secs(5);
        let mut respawned_pid = None;
        while Instant::now() < deadline {
            manager.ensure_plugin_daemon_running(PLUGIN_ID).unwrap();
            let pid = manager.get(PLUGIN_ID).unwrap().daemon_pid();
            if pid.is_some() && pid != Some(old_pid) {
                respawned_pid = pid;
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }

        assert!(
            respawned_pid.is_some(),
            "daemon must respawn with a fresh pid after it dies, still stuck on pid {}",
            old_pid
        );

        manager
            .plugins
            .get_mut(PLUGIN_ID)
            .unwrap()
            .stop_daemon()
            .unwrap();
    }
}
