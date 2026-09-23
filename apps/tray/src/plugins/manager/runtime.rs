use super::{autostart, loading, PluginManager};
use crate::plugins::Plugin;
use anyhow::Result;

pub(super) fn reload_plugins(manager: &mut PluginManager) -> Result<()> {
    log::info!("Reloading all plugins...");
    stop_all_plugins(manager);
    loading::load_plugins(manager)?;
    manager.autostart_daemons();
    Ok(())
}

pub(super) fn reload_plugin(manager: &mut PluginManager, plugin_id: &str) -> Result<u64> {
    log::info!("Reloading plugin: {plugin_id}");
    let requested_generation = crate::plugins::config::current_profile_config_generation();
    qol_runtime::probe!(
        "PLUGIN_RELOAD",
        "plugin={plugin_id} stage=start scope=single requested_generation={requested_generation} consumed_generation=none acknowledged_generation=none"
    );

    let loaded = loading::load_plugin(plugin_id)?;
    #[cfg(debug_assertions)]
    let old_pid = manager.plugins.get(plugin_id).and_then(Plugin::daemon_pid);
    #[cfg(not(debug_assertions))]
    let old_pid = None::<u32>;
    manager.process_tracker().kill_plugin_processes(plugin_id);
    drop(manager.plugins.remove(plugin_id));

    #[cfg(debug_assertions)]
    let loaded_plugin = loaded.plugin.is_some();
    #[cfg(not(debug_assertions))]
    let loaded_plugin = false;
    let mut consumed_generation = requested_generation;
    if let Some(plugin) = loaded.plugin {
        if !crate::dev_generation::is_shadow() && !crate::dev_generation::is_rolling_restart() {
            super::super::daemon_tracker::clean_stale_sockets(std::slice::from_ref(&plugin));
        }
        let id = plugin.id.clone();
        manager.plugins.insert(id.clone(), plugin);
        if let Some(plugin) = manager.plugins.get_mut(&id) {
            consumed_generation = autostart::start_plugin_daemons(
                std::iter::once(&mut *plugin),
                Some(&manager.lifecycle_cancellation),
            )
            .first()
            .map(|(_, generation)| *generation)
            .unwrap_or(requested_generation);
        }
    }

    loading::rebuild_identity_index(manager);
    manager.set_resolution_report(loaded.report);
    sync_ignore_pids(manager);
    #[cfg(debug_assertions)]
    let new_pid = manager.plugins.get(plugin_id).and_then(Plugin::daemon_pid);
    #[cfg(not(debug_assertions))]
    let new_pid = None::<u32>;
    qol_runtime::probe!(
        "PLUGIN_RELOAD",
        "plugin={plugin_id} stage=done scope=single loaded={loaded_plugin} old_pid={old_pid:?} new_pid={new_pid:?} consumed_generation={consumed_generation} acknowledged_generation=none"
    );
    Ok(consumed_generation)
}

pub(super) fn shutdown(manager: &mut PluginManager) {
    log::info!("Shutting down plugins...");
    crate::settings_surface::stop();
    stop_all_plugins(manager);
}

pub(super) fn restart_running_plugin_daemon(
    manager: &mut PluginManager,
    plugin_id: &str,
) -> Result<()> {
    if manager.reconcile_profile_generation()? {
        sync_ignore_pids(manager);
        return Ok(());
    }
    {
        let plugin = plugin_mut(manager, plugin_id)?;
        if plugin.daemon_pid().is_none() {
            return Ok(());
        }
        plugin.stop_daemon()?;
    }
    start_plugin_daemon_with_current_config(manager, plugin_id)?;
    sync_ignore_pids(manager);
    Ok(())
}

pub(super) fn ensure_plugin_daemon_running(
    manager: &mut PluginManager,
    plugin_id: &str,
) -> Result<()> {
    manager.reconcile_profile_generation()?;
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
        super::super::daemon_lifecycle::track_desktop_state_pid(pid);
    }
}

fn stop_all_plugins(manager: &mut PluginManager) {
    manager.process_tracker().kill_all_plugin_processes();
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
    crate::runtime::PluginStatusRegistry::shared().clear(plugin.id.as_str());
}

fn start_plugin_daemon_if_needed(manager: &mut PluginManager, plugin_id: &str) -> Result<()> {
    if crate::dev_generation::daemon_autostart_held() {
        return Ok(());
    }
    {
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
    }

    start_plugin_daemon_with_current_config(manager, plugin_id)
}

fn start_plugin_daemon_with_current_config(
    manager: &mut PluginManager,
    plugin_id: &str,
) -> Result<()> {
    let plugin = plugin_mut(manager, plugin_id)?;
    let mut runtime_config = crate::plugins::config::RuntimeConfigContext::new()?;
    let consumed_generation = super::super::daemon_lifecycle::start_daemon_with_context(
        plugin,
        Some(&mut runtime_config),
    )?;
    if let Some(generation) = consumed_generation {
        manager.acknowledge_profile_plugin_generation(plugin_id, generation);
    }
    Ok(())
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
    use crate::plugins::{PluginId, PluginLoader};
    use std::time::{Duration, Instant};

    const PLUGIN_ID: &str = "plugin-foo";

    fn write_daemon_plugin(
        plugins_dir: &std::path::Path,
        plugin_id: &str,
        version: &str,
    ) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;

        let plugin_dir = plugins_dir.join(plugin_id);
        std::fs::create_dir_all(&plugin_dir).unwrap();
        let script = plugin_dir.join("daemon");
        std::fs::write(&script, "#!/bin/sh\nexec sleep 30\n").unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::write(
            plugin_dir.join("plugin.toml"),
            format!(
                r#"
[plugin]
id = "{plugin_id}"
name = "{plugin_id}"
description = ""
version = "{version}"

[menu]
label = "{plugin_id}"
items = []

[daemon]
enabled = true
command = "daemon"
"#,
            ),
        )
        .unwrap();
        plugin_dir
    }

    fn insert_loaded_plugin(manager: &mut PluginManager, plugin_id: &str, path: &std::path::Path) {
        let plugin = PluginLoader::load_plugin_with_id(plugin_id, path).unwrap();
        manager.plugins.insert(PluginId::new(plugin_id), plugin);
        manager.ensure_plugin_daemon_running(plugin_id).unwrap();
    }

    #[test]
    fn ensure_plugin_daemon_running_respawns_daemon_after_unexpected_exit() {
        let _env_lock = crate::test_support::env_lock().blocking_lock();
        let _runtime_cache_lock = crate::test_support::runtime_cache_lock().blocking_lock();
        let root = tempfile::TempDir::new().unwrap();
        let _guard = crate::paths::push_test_path_root(root.path());
        let plugins_dir = crate::paths::plugins_dir().unwrap();
        let plugin_dir = write_daemon_plugin(&plugins_dir, PLUGIN_ID, "1.0.0");
        let config_dir = crate::paths::shared_config_dir().unwrap();
        crate::plugins::registry::record_release_install(&config_dir, PLUGIN_ID, plugin_dir)
            .unwrap();
        let mut manager = PluginManager::new();
        insert_loaded_plugin(&mut manager, PLUGIN_ID, &plugins_dir.join(PLUGIN_ID));
        assert_eq!(manager.profile_reconciliation_count(), 0);
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
        assert_eq!(
            manager.profile_reconciliation_count(),
            0,
            "liveness respawn must not be satisfied by profile reconciliation"
        );

        manager
            .plugins
            .get_mut(PLUGIN_ID)
            .unwrap()
            .stop_daemon()
            .unwrap();
    }

    #[test]
    fn ensure_plugin_daemon_running_reconciles_applied_profile_generation() {
        let _env = crate::test_support::env_lock().blocking_lock();
        let root = tempfile::TempDir::new().unwrap();
        let _guard = crate::paths::push_test_path_root(root.path());
        let plugins_dir = crate::paths::plugins_dir().unwrap();
        let plugin_dir = write_daemon_plugin(&plugins_dir, PLUGIN_ID, "1.0.0");
        let config_dir = crate::paths::shared_config_dir().unwrap();
        crate::plugins::registry::record_release_install(&config_dir, PLUGIN_ID, plugin_dir)
            .unwrap();

        let mut manager = PluginManager::new();
        insert_loaded_plugin(&mut manager, PLUGIN_ID, &plugins_dir.join(PLUGIN_ID));
        let old_pid = manager.get(PLUGIN_ID).unwrap().daemon_pid().unwrap();

        let profile_configs = crate::paths::profile_os_dir()
            .unwrap()
            .join(crate::features::profile::scope_store::PLUGIN_CONFIGS_SUBDIR);
        {
            let _profile_guard = crate::plugins::config::profile_config_write_guard();
            std::fs::create_dir_all(&profile_configs).unwrap();
            std::fs::write(profile_configs.join(format!("{PLUGIN_ID}.json")), "{}\n").unwrap();
        }

        manager.ensure_plugin_daemon_running(PLUGIN_ID).unwrap();
        let new_pid = manager.get(PLUGIN_ID).unwrap().daemon_pid().unwrap();
        assert_ne!(
            new_pid, old_pid,
            "an applied profile change must restart the daemon"
        );
        assert!(
            !crate::process_utils::is_pid_alive(old_pid as i32),
            "the old-generation daemon must not survive reconciliation"
        );

        manager
            .plugins
            .get_mut(PLUGIN_ID)
            .unwrap()
            .stop_daemon()
            .unwrap();
    }

    #[test]
    fn scoped_profile_generation_reloads_only_affected_daemon() {
        const TARGET_ID: &str = "plugin-scoped-generation-target";
        const OTHER_ID: &str = "plugin-scoped-generation-other";

        let _env = crate::test_support::env_lock().blocking_lock();
        let root = tempfile::TempDir::new().unwrap();
        let _guard = crate::paths::push_test_path_root(root.path());
        let plugins_dir = crate::paths::plugins_dir().unwrap();
        let target_dir = write_daemon_plugin(&plugins_dir, TARGET_ID, "1.0.0");
        let other_dir = write_daemon_plugin(&plugins_dir, OTHER_ID, "1.0.0");
        let config_dir = crate::paths::shared_config_dir().unwrap();
        crate::plugins::registry::record_release_install(
            &config_dir,
            TARGET_ID,
            target_dir.clone(),
        )
        .unwrap();
        crate::plugins::registry::record_release_install(&config_dir, OTHER_ID, other_dir.clone())
            .unwrap();

        let mut manager = PluginManager::new();
        insert_loaded_plugin(&mut manager, TARGET_ID, &target_dir);
        insert_loaded_plugin(&mut manager, OTHER_ID, &other_dir);
        let target_pid_before = manager.get(TARGET_ID).unwrap().daemon_pid().unwrap();
        let other_pid_before = manager.get(OTHER_ID).unwrap().daemon_pid().unwrap();

        crate::plugins::PluginConfigManager::new()
            .unwrap()
            .set_config(TARGET_ID, serde_json::json!({"enabled": true}))
            .unwrap();

        assert!(manager.reconcile_profile_generation().unwrap());
        let target_pid_after = manager.get(TARGET_ID).unwrap().daemon_pid().unwrap();
        assert_ne!(target_pid_after, target_pid_before);
        assert_eq!(
            manager.get(OTHER_ID).unwrap().daemon_pid(),
            Some(other_pid_before),
            "an unrelated profile mutation must not restart the other daemon"
        );
        assert!(!manager.reconcile_profile_generation().unwrap());

        for plugin in manager.plugins.values_mut() {
            plugin.stop_daemon().unwrap();
        }
    }

    #[test]
    fn targeted_profile_lifecycle_reload_is_not_repeated_after_lock_sync() {
        const TARGET_ID: &str = "plugin-targeted-lifecycle-target";
        const OTHER_ID: &str = "plugin-targeted-lifecycle-other";

        let _env = crate::test_support::env_lock().blocking_lock();
        let root = tempfile::TempDir::new().unwrap();
        let _guard = crate::paths::push_test_path_root(root.path());
        let plugins_dir = crate::paths::plugins_dir().unwrap();
        let target_dir = write_daemon_plugin(&plugins_dir, TARGET_ID, "1.0.0");
        let other_dir = write_daemon_plugin(&plugins_dir, OTHER_ID, "1.0.0");
        let config_dir = crate::paths::shared_config_dir().unwrap();
        crate::plugins::registry::record_release_install(
            &config_dir,
            TARGET_ID,
            target_dir.clone(),
        )
        .unwrap();
        crate::plugins::registry::record_release_install(&config_dir, OTHER_ID, other_dir.clone())
            .unwrap();
        crate::features::profile::core::save_plugins_lock(
            &crate::features::profile::core::PluginsLock {
                version: crate::features::profile::core::CURRENT_PROFILE_VERSION,
                plugins: vec![
                    crate::features::profile::core::PluginLockEntry {
                        uid: crate::plugins::PluginUid::new(TARGET_ID),
                        id: TARGET_ID.to_string(),
                        repo_url: format!("https://example.com/{TARGET_ID}.git"),
                        version: "1.0.0".to_string(),
                        platforms: None,
                    },
                    crate::features::profile::core::PluginLockEntry {
                        uid: crate::plugins::PluginUid::new(OTHER_ID),
                        id: OTHER_ID.to_string(),
                        repo_url: format!("https://example.com/{OTHER_ID}.git"),
                        version: "1.0.0".to_string(),
                        platforms: None,
                    },
                ],
            },
        )
        .unwrap();

        let mut manager = PluginManager::new();
        insert_loaded_plugin(&mut manager, TARGET_ID, &target_dir);
        insert_loaded_plugin(&mut manager, OTHER_ID, &other_dir);
        let other_pid = manager.get(OTHER_ID).unwrap().daemon_pid().unwrap();

        write_daemon_plugin(&plugins_dir, TARGET_ID, "2.0.0");
        manager.reload_plugin(TARGET_ID).unwrap();
        let target_pid = manager.get(TARGET_ID).unwrap().daemon_pid().unwrap();
        assert_eq!(
            manager.get(TARGET_ID).unwrap().manifest.plugin.version,
            "2.0.0"
        );

        let (_, lock_generation) =
            crate::features::profile::core::sync_plugins_lock_from_plugins_with_generation(
                manager.plugins(),
            )
            .unwrap();
        manager.acknowledge_profile_plugin_generation(TARGET_ID, lock_generation);

        assert!(!manager.reconcile_profile_generation().unwrap());
        assert_eq!(
            manager.get(TARGET_ID).unwrap().daemon_pid(),
            Some(target_pid),
            "targeted install/update/uninstall reload must not be repeated by the event"
        );
        assert_eq!(manager.get(OTHER_ID).unwrap().daemon_pid(), Some(other_pid));
        assert_eq!(manager.profile_reconciliation_count(), 1);

        for plugin in manager.plugins.values_mut() {
            plugin.stop_daemon().unwrap();
        }
    }

    #[test]
    fn profile_write_after_reload_is_reconciled_after_token_acknowledgement() {
        const PLUGIN_ID: &str = "plugin-reload-ack-interleaving";

        let _env = crate::test_support::env_lock().blocking_lock();
        let root = tempfile::TempDir::new().unwrap();
        let _guard = crate::paths::push_test_path_root(root.path());
        let plugins_dir = crate::paths::plugins_dir().unwrap();
        let plugin_dir = write_daemon_plugin(&plugins_dir, PLUGIN_ID, "1.0.0");
        let config_dir = crate::paths::shared_config_dir().unwrap();
        crate::plugins::registry::record_release_install(
            &config_dir,
            PLUGIN_ID,
            plugin_dir.clone(),
        )
        .unwrap();

        let mut manager = PluginManager::new();
        insert_loaded_plugin(&mut manager, PLUGIN_ID, &plugin_dir);
        let consumed_generation = super::reload_plugin(&mut manager, PLUGIN_ID).unwrap();
        let reloaded_pid = manager.get(PLUGIN_ID).unwrap().daemon_pid().unwrap();

        {
            let _profile_guard =
                crate::plugins::config::profile_config_write_guard_for_plugin(PLUGIN_ID);
        }
        manager.acknowledge_profile_plugin_generation(PLUGIN_ID, consumed_generation);

        assert!(manager.reconcile_profile_generation().unwrap());
        let reconciled_pid = manager.get(PLUGIN_ID).unwrap().daemon_pid().unwrap();
        assert_ne!(reconciled_pid, reloaded_pid);
        assert_eq!(manager.profile_reconciliation_count(), 1);

        manager
            .plugins
            .get_mut(PLUGIN_ID)
            .unwrap()
            .stop_daemon()
            .unwrap();
    }

    #[test]
    fn reload_plugin_restarts_only_the_target_daemon() {
        let _runtime_cache_lock = crate::test_support::runtime_cache_lock().blocking_lock();
        const TARGET_ID: &str = "plugin-selective-reload-target";
        const OTHER_ID: &str = "plugin-selective-reload-other";

        let root = tempfile::TempDir::new().unwrap();
        let _guard = crate::paths::push_test_path_root(root.path());
        let plugins_dir = crate::paths::plugins_dir().unwrap();
        let target_dir = write_daemon_plugin(&plugins_dir, TARGET_ID, "1.0.0");
        let other_dir = write_daemon_plugin(&plugins_dir, OTHER_ID, "1.0.0");
        let config_dir = crate::paths::shared_config_dir().unwrap();
        crate::plugins::registry::record_release_install(
            &config_dir,
            TARGET_ID,
            target_dir.clone(),
        )
        .unwrap();
        crate::plugins::registry::record_release_install(&config_dir, OTHER_ID, other_dir.clone())
            .unwrap();

        let mut manager = PluginManager::new();
        insert_loaded_plugin(&mut manager, TARGET_ID, &target_dir);
        insert_loaded_plugin(&mut manager, OTHER_ID, &other_dir);
        let target_pid_before = manager.get(TARGET_ID).unwrap().daemon_pid().unwrap();
        let other_pid_before = manager.get(OTHER_ID).unwrap().daemon_pid().unwrap();

        write_daemon_plugin(&plugins_dir, TARGET_ID, "2.0.0");
        manager.reload_plugin(TARGET_ID).unwrap();

        let target = manager.get(TARGET_ID).expect("target remains loaded");
        let target_pid_after = target.daemon_pid().expect("target daemon restarted");
        assert_eq!(target.manifest.plugin.version, "2.0.0");
        assert_ne!(target_pid_after, target_pid_before);
        assert!(
            !crate::process_utils::is_pid_alive(target_pid_before as i32),
            "the replaced target daemon must be stopped"
        );
        assert_eq!(
            manager.get(OTHER_ID).unwrap().daemon_pid(),
            Some(other_pid_before),
            "an unrelated plugin daemon must keep the exact same process"
        );
        assert!(
            crate::process_utils::is_pid_alive(other_pid_before as i32),
            "the unrelated daemon must remain alive"
        );

        for plugin in manager.plugins.values_mut() {
            plugin.stop_daemon().unwrap();
        }
    }

    use crate::plugins::manager::reload_delivery::{
        CompletionDecision, PendingReloadTicket, ReloadDeliveryOutcome, RELOAD_DELIVERY_TTL,
    };

    fn tracked_config_save(
        plugin_id: &str,
        value: u64,
    ) -> crate::plugins::config::ConfigSaveReceipt {
        crate::plugins::PluginConfigManager::new()
            .unwrap()
            .set_config_tracked(plugin_id, serde_json::json!({ "value": value }))
            .unwrap()
    }

    fn begin_and_record_reload(
        manager: &mut PluginManager,
        plugin_id: &str,
        value: u64,
    ) -> (
        PendingReloadTicket,
        crate::plugins::config::ConfigSaveReceipt,
    ) {
        let ticket = manager.begin_config_reload(plugin_id).unwrap();
        let receipt = tracked_config_save(plugin_id, value);
        manager
            .record_config_reload_saved(&ticket, receipt)
            .unwrap();
        (ticket, receipt)
    }

    fn spawn_fixture_plugin(
        manager: &mut PluginManager,
        plugins_dir: &std::path::Path,
        plugin_id: &str,
    ) -> u32 {
        let plugin_dir = write_daemon_plugin(plugins_dir, plugin_id, "1.0.0");
        let config_dir = crate::paths::shared_config_dir().unwrap();
        crate::plugins::registry::record_release_install(&config_dir, plugin_id, plugin_dir)
            .unwrap();
        insert_loaded_plugin(manager, plugin_id, &plugins_dir.join(plugin_id));
        manager.get(plugin_id).unwrap().daemon_pid().unwrap()
    }

    struct ReloadClock(Instant);

    impl ReloadClock {
        fn start(instant: Instant) -> Self {
            crate::plugins::manager::reload_delivery::set_test_now(Some(instant));
            Self(instant)
        }

        fn advance(&self, duration: Duration) {
            crate::plugins::manager::reload_delivery::set_test_now(Some(self.0 + duration));
        }
    }

    impl Drop for ReloadClock {
        fn drop(&mut self) {
            crate::plugins::manager::reload_delivery::set_test_now(None);
        }
    }

    #[test]
    fn handled_config_reload_acknowledges_the_exact_save_and_keeps_the_daemon() {
        let _env_lock = crate::test_support::env_lock().blocking_lock();
        let _runtime_cache_lock = crate::test_support::runtime_cache_lock().blocking_lock();
        let root = tempfile::TempDir::new().unwrap();
        let _guard = crate::paths::push_test_path_root(root.path());
        let plugins_dir = crate::paths::plugins_dir().unwrap();
        let mut manager = PluginManager::new();
        let pid_before = spawn_fixture_plugin(&mut manager, &plugins_dir, PLUGIN_ID);

        let (ticket, receipt) = begin_and_record_reload(&mut manager, PLUGIN_ID, 1);
        assert_eq!(
            manager.complete_config_reload(&ticket, ReloadDeliveryOutcome::Handled),
            CompletionDecision::Acknowledged {
                generation: receipt.generation
            }
        );
        assert_eq!(
            manager.acknowledged_plugin_generation(PLUGIN_ID),
            Some(receipt.generation),
            "the acknowledgement must name the generation this save published"
        );

        assert!(!manager.reconcile_profile_generation().unwrap());
        assert_eq!(
            manager.get(PLUGIN_ID).unwrap().daemon_pid(),
            Some(pid_before),
            "an acknowledged reload must not restart the daemon"
        );

        manager
            .plugins
            .get_mut(PLUGIN_ID)
            .unwrap()
            .stop_daemon()
            .unwrap();
    }

    #[test]
    fn pending_config_reload_defers_reconciliation_until_acknowledgement() {
        let _env_lock = crate::test_support::env_lock().blocking_lock();
        let _runtime_cache_lock = crate::test_support::runtime_cache_lock().blocking_lock();
        let root = tempfile::TempDir::new().unwrap();
        let _guard = crate::paths::push_test_path_root(root.path());
        let plugins_dir = crate::paths::plugins_dir().unwrap();
        let mut manager = PluginManager::new();
        let pid_before = spawn_fixture_plugin(&mut manager, &plugins_dir, PLUGIN_ID);

        let (ticket, receipt) = begin_and_record_reload(&mut manager, PLUGIN_ID, 1);
        assert!(manager.has_pending_config_reload(PLUGIN_ID));
        assert!(!manager.reconcile_profile_generation().unwrap());
        assert_eq!(
            manager.get(PLUGIN_ID).unwrap().daemon_pid(),
            Some(pid_before),
            "reconciliation must not restart a daemon with an in-flight save"
        );

        assert_eq!(
            manager.complete_config_reload(&ticket, ReloadDeliveryOutcome::Handled),
            CompletionDecision::Acknowledged {
                generation: receipt.generation
            }
        );
        assert!(!manager.has_pending_config_reload(PLUGIN_ID));
        assert!(!manager.reconcile_profile_generation().unwrap());
        assert_eq!(
            manager.get(PLUGIN_ID).unwrap().daemon_pid(),
            Some(pid_before)
        );

        manager
            .plugins
            .get_mut(PLUGIN_ID)
            .unwrap()
            .stop_daemon()
            .unwrap();
    }

    #[test]
    fn completing_one_config_reload_leaves_the_other_request_pending() {
        let _env_lock = crate::test_support::env_lock().blocking_lock();
        let _runtime_cache_lock = crate::test_support::runtime_cache_lock().blocking_lock();
        let root = tempfile::TempDir::new().unwrap();
        let _guard = crate::paths::push_test_path_root(root.path());
        let plugins_dir = crate::paths::plugins_dir().unwrap();
        let mut manager = PluginManager::new();
        let pid_before = spawn_fixture_plugin(&mut manager, &plugins_dir, PLUGIN_ID);

        let (first, first_receipt) = begin_and_record_reload(&mut manager, PLUGIN_ID, 1);
        let (second, second_receipt) = begin_and_record_reload(&mut manager, PLUGIN_ID, 2);

        assert_eq!(
            manager.complete_config_reload(&first, ReloadDeliveryOutcome::Handled),
            CompletionDecision::Acknowledged {
                generation: first_receipt.generation
            }
        );
        assert!(
            manager.has_pending_config_reload(PLUGIN_ID),
            "completing one request must leave the newer request pending"
        );
        assert!(!manager.reconcile_profile_generation().unwrap());
        assert_eq!(
            manager.get(PLUGIN_ID).unwrap().daemon_pid(),
            Some(pid_before)
        );

        assert_eq!(
            manager.complete_config_reload(&second, ReloadDeliveryOutcome::Handled),
            CompletionDecision::Acknowledged {
                generation: second_receipt.generation
            }
        );
        assert!(!manager.has_pending_config_reload(PLUGIN_ID));
        assert!(!manager.reconcile_profile_generation().unwrap());
        assert_eq!(
            manager.get(PLUGIN_ID).unwrap().daemon_pid(),
            Some(pid_before)
        );

        manager
            .plugins
            .get_mut(PLUGIN_ID)
            .unwrap()
            .stop_daemon()
            .unwrap();
    }

    #[test]
    fn reversed_config_reload_completions_keep_the_latest_generation_authoritative() {
        let _env_lock = crate::test_support::env_lock().blocking_lock();
        let _runtime_cache_lock = crate::test_support::runtime_cache_lock().blocking_lock();
        let root = tempfile::TempDir::new().unwrap();
        let _guard = crate::paths::push_test_path_root(root.path());
        let plugins_dir = crate::paths::plugins_dir().unwrap();
        let mut manager = PluginManager::new();
        let pid_before = spawn_fixture_plugin(&mut manager, &plugins_dir, PLUGIN_ID);

        let (first, first_receipt) = begin_and_record_reload(&mut manager, PLUGIN_ID, 1);
        let (second, second_receipt) = begin_and_record_reload(&mut manager, PLUGIN_ID, 2);
        assert!(second_receipt.generation > first_receipt.generation);

        assert_eq!(
            manager.complete_config_reload(&second, ReloadDeliveryOutcome::Handled),
            CompletionDecision::Acknowledged {
                generation: second_receipt.generation
            }
        );
        assert_eq!(
            manager.complete_config_reload(&first, ReloadDeliveryOutcome::Handled),
            CompletionDecision::Acknowledged {
                generation: first_receipt.generation
            }
        );
        assert_eq!(
            manager.acknowledged_plugin_generation(PLUGIN_ID),
            Some(second_receipt.generation),
            "a late older acknowledgement must not regress the watermark"
        );
        assert!(!manager.reconcile_profile_generation().unwrap());
        assert_eq!(
            manager.get(PLUGIN_ID).unwrap().daemon_pid(),
            Some(pid_before)
        );

        manager
            .plugins
            .get_mut(PLUGIN_ID)
            .unwrap()
            .stop_daemon()
            .unwrap();
    }

    #[test]
    fn failed_config_save_retires_its_request_without_suppressing_reconciliation() {
        let _env_lock = crate::test_support::env_lock().blocking_lock();
        let _runtime_cache_lock = crate::test_support::runtime_cache_lock().blocking_lock();
        let root = tempfile::TempDir::new().unwrap();
        let _guard = crate::paths::push_test_path_root(root.path());
        let plugins_dir = crate::paths::plugins_dir().unwrap();
        let mut manager = PluginManager::new();
        let pid_before = spawn_fixture_plugin(&mut manager, &plugins_dir, PLUGIN_ID);

        let ticket = manager.begin_config_reload(PLUGIN_ID).unwrap();
        {
            let _published =
                crate::plugins::config::profile_config_write_guard_for_plugin(PLUGIN_ID);
        }
        assert_eq!(
            manager.complete_config_reload(&ticket, ReloadDeliveryOutcome::SaveFailed),
            CompletionDecision::Superseded
        );

        assert!(
            manager.reconcile_profile_generation().unwrap(),
            "an unpublished or failed save must stay eligible for reconciliation"
        );
        let pid_after = manager.get(PLUGIN_ID).unwrap().daemon_pid().unwrap();
        assert_ne!(pid_after, pid_before);
        assert_eq!(
            manager.complete_config_reload(&ticket, ReloadDeliveryOutcome::Handled),
            CompletionDecision::Superseded
        );
        assert_eq!(
            manager.get(PLUGIN_ID).unwrap().daemon_pid(),
            Some(pid_after)
        );

        manager
            .plugins
            .get_mut(PLUGIN_ID)
            .unwrap()
            .stop_daemon()
            .unwrap();
    }

    #[test]
    fn expired_config_reload_is_revisited_without_another_save() {
        let _env_lock = crate::test_support::env_lock().blocking_lock();
        let _runtime_cache_lock = crate::test_support::runtime_cache_lock().blocking_lock();
        let root = tempfile::TempDir::new().unwrap();
        let _guard = crate::paths::push_test_path_root(root.path());
        let plugins_dir = crate::paths::plugins_dir().unwrap();
        let mut manager = PluginManager::new();
        let pid_before = spawn_fixture_plugin(&mut manager, &plugins_dir, PLUGIN_ID);
        let clock = ReloadClock::start(Instant::now());

        let (ticket, _receipt) = begin_and_record_reload(&mut manager, PLUGIN_ID, 1);
        assert!(!manager.reconcile_profile_generation().unwrap());
        assert_eq!(
            manager.get(PLUGIN_ID).unwrap().daemon_pid(),
            Some(pid_before)
        );

        clock.advance(RELOAD_DELIVERY_TTL + Duration::from_secs(1));
        assert!(
            manager.reconcile_profile_generation().unwrap(),
            "an expired request must be revisited without another configuration mutation"
        );
        let pid_after = manager.get(PLUGIN_ID).unwrap().daemon_pid().unwrap();
        assert_ne!(pid_after, pid_before);
        assert!(!manager.has_pending_config_reload(PLUGIN_ID));
        assert_eq!(
            manager.complete_config_reload(&ticket, ReloadDeliveryOutcome::Handled),
            CompletionDecision::Superseded
        );
        assert_eq!(
            manager.get(PLUGIN_ID).unwrap().daemon_pid(),
            Some(pid_after)
        );

        drop(clock);
        manager
            .plugins
            .get_mut(PLUGIN_ID)
            .unwrap()
            .stop_daemon()
            .unwrap();
    }

    #[test]
    fn replaced_daemon_rejects_stale_completion_and_failure() {
        let _env_lock = crate::test_support::env_lock().blocking_lock();
        let _runtime_cache_lock = crate::test_support::runtime_cache_lock().blocking_lock();
        let root = tempfile::TempDir::new().unwrap();
        let _guard = crate::paths::push_test_path_root(root.path());
        let plugins_dir = crate::paths::plugins_dir().unwrap();
        let mut manager = PluginManager::new();
        let pid_before = spawn_fixture_plugin(&mut manager, &plugins_dir, PLUGIN_ID);

        let (ticket, receipt) = begin_and_record_reload(&mut manager, PLUGIN_ID, 1);
        manager.restart_running_plugin_daemon(PLUGIN_ID).unwrap();
        let pid_after = manager.get(PLUGIN_ID).unwrap().daemon_pid().unwrap();
        assert_ne!(pid_after, pid_before);

        assert_eq!(
            manager.complete_config_reload(&ticket, ReloadDeliveryOutcome::Handled),
            CompletionDecision::Superseded,
            "the replaced daemon cannot acknowledge the old request"
        );
        assert_eq!(
            manager.complete_config_reload(&ticket, ReloadDeliveryOutcome::Failed),
            CompletionDecision::Superseded,
            "a stale failure must not restart the replacement daemon"
        );
        assert_eq!(
            manager.get(PLUGIN_ID).unwrap().daemon_pid(),
            Some(pid_after)
        );
        assert!(
            manager
                .acknowledged_plugin_generation(PLUGIN_ID)
                .is_some_and(|generation| generation >= receipt.generation),
            "the replacement daemon must have recorded the generation it consumed"
        );

        manager
            .plugins
            .get_mut(PLUGIN_ID)
            .unwrap()
            .stop_daemon()
            .unwrap();
    }

    #[test]
    fn failed_older_config_reload_does_not_restart_while_a_newer_request_is_pending() {
        let _env_lock = crate::test_support::env_lock().blocking_lock();
        let _runtime_cache_lock = crate::test_support::runtime_cache_lock().blocking_lock();
        let root = tempfile::TempDir::new().unwrap();
        let _guard = crate::paths::push_test_path_root(root.path());
        let plugins_dir = crate::paths::plugins_dir().unwrap();
        let mut manager = PluginManager::new();
        let pid_before = spawn_fixture_plugin(&mut manager, &plugins_dir, PLUGIN_ID);

        let (older, _older_receipt) = begin_and_record_reload(&mut manager, PLUGIN_ID, 1);
        let (newer, newer_receipt) = begin_and_record_reload(&mut manager, PLUGIN_ID, 2);

        assert_eq!(
            manager.complete_config_reload(&older, ReloadDeliveryOutcome::Failed),
            CompletionDecision::Superseded,
            "an older failure must not discard the newer in-flight save"
        );
        assert_eq!(
            manager.get(PLUGIN_ID).unwrap().daemon_pid(),
            Some(pid_before)
        );
        assert!(manager.has_pending_config_reload(PLUGIN_ID));

        assert_eq!(
            manager.complete_config_reload(&newer, ReloadDeliveryOutcome::Handled),
            CompletionDecision::Acknowledged {
                generation: newer_receipt.generation
            }
        );
        assert!(!manager.reconcile_profile_generation().unwrap());
        assert_eq!(
            manager.get(PLUGIN_ID).unwrap().daemon_pid(),
            Some(pid_before)
        );

        manager
            .plugins
            .get_mut(PLUGIN_ID)
            .unwrap()
            .stop_daemon()
            .unwrap();
    }

    #[test]
    fn failed_reload_after_newer_acknowledged_generation_does_not_restart() {
        let _env_lock = crate::test_support::env_lock().blocking_lock();
        let _runtime_cache_lock = crate::test_support::runtime_cache_lock().blocking_lock();
        let root = tempfile::TempDir::new().unwrap();
        let _guard = crate::paths::push_test_path_root(root.path());
        let plugins_dir = crate::paths::plugins_dir().unwrap();
        let mut manager = PluginManager::new();
        let pid_before = spawn_fixture_plugin(&mut manager, &plugins_dir, PLUGIN_ID);

        let (older, _older_receipt) = begin_and_record_reload(&mut manager, PLUGIN_ID, 1);
        let (newer, newer_receipt) = begin_and_record_reload(&mut manager, PLUGIN_ID, 2);
        assert_eq!(
            manager.complete_config_reload(&newer, ReloadDeliveryOutcome::Handled),
            CompletionDecision::Acknowledged {
                generation: newer_receipt.generation
            }
        );
        assert_eq!(
            manager.complete_config_reload(&older, ReloadDeliveryOutcome::Failed),
            CompletionDecision::Superseded,
            "a failed request must not restart after a newer save was consumed"
        );
        assert_eq!(
            manager.get(PLUGIN_ID).unwrap().daemon_pid(),
            Some(pid_before)
        );
        assert!(!manager.reconcile_profile_generation().unwrap());
        assert_eq!(
            manager.get(PLUGIN_ID).unwrap().daemon_pid(),
            Some(pid_before)
        );

        manager
            .plugins
            .get_mut(PLUGIN_ID)
            .unwrap()
            .stop_daemon()
            .unwrap();
    }

    #[test]
    fn failed_config_reload_notification_restarts_and_reconciles_cleanly() {
        let _env_lock = crate::test_support::env_lock().blocking_lock();
        let _runtime_cache_lock = crate::test_support::runtime_cache_lock().blocking_lock();
        let root = tempfile::TempDir::new().unwrap();
        let _guard = crate::paths::push_test_path_root(root.path());
        let plugins_dir = crate::paths::plugins_dir().unwrap();
        let mut manager = PluginManager::new();
        let pid_before = spawn_fixture_plugin(&mut manager, &plugins_dir, PLUGIN_ID);

        let (ticket, _receipt) = begin_and_record_reload(&mut manager, PLUGIN_ID, 1);
        assert_eq!(
            manager.complete_config_reload(&ticket, ReloadDeliveryOutcome::Failed),
            CompletionDecision::Restarted
        );
        let pid_after = manager.get(PLUGIN_ID).unwrap().daemon_pid().unwrap();
        assert_ne!(pid_after, pid_before);
        assert!(
            !manager.reconcile_profile_generation().unwrap(),
            "the replacement daemon must have consumed the saved generation"
        );
        assert_eq!(
            manager.get(PLUGIN_ID).unwrap().daemon_pid(),
            Some(pid_after)
        );

        manager
            .plugins
            .get_mut(PLUGIN_ID)
            .unwrap()
            .stop_daemon()
            .unwrap();
    }

    #[test]
    fn pending_config_reload_does_not_block_unrelated_plugin_reconciliation() {
        const TARGET_ID: &str = "plugin-pending-reload-target";
        const OTHER_ID: &str = "plugin-pending-reload-other";

        let _env_lock = crate::test_support::env_lock().blocking_lock();
        let _runtime_cache_lock = crate::test_support::runtime_cache_lock().blocking_lock();
        let root = tempfile::TempDir::new().unwrap();
        let _guard = crate::paths::push_test_path_root(root.path());
        let plugins_dir = crate::paths::plugins_dir().unwrap();
        let mut manager = PluginManager::new();
        let target_pid = spawn_fixture_plugin(&mut manager, &plugins_dir, TARGET_ID);
        let other_pid = spawn_fixture_plugin(&mut manager, &plugins_dir, OTHER_ID);

        let (ticket, receipt) = begin_and_record_reload(&mut manager, TARGET_ID, 1);
        crate::plugins::PluginConfigManager::new()
            .unwrap()
            .set_config(OTHER_ID, serde_json::json!({"enabled": true}))
            .unwrap();

        assert!(manager.reconcile_profile_generation().unwrap());
        assert_eq!(
            manager.get(TARGET_ID).unwrap().daemon_pid(),
            Some(target_pid),
            "the pending plugin must stay deferred"
        );
        assert_ne!(
            manager.get(OTHER_ID).unwrap().daemon_pid(),
            Some(other_pid),
            "an unrelated plugin must still reconcile"
        );

        assert_eq!(
            manager.complete_config_reload(&ticket, ReloadDeliveryOutcome::Handled),
            CompletionDecision::Acknowledged {
                generation: receipt.generation
            }
        );
        assert!(!manager.reconcile_profile_generation().unwrap());
        assert_eq!(
            manager.get(TARGET_ID).unwrap().daemon_pid(),
            Some(target_pid)
        );

        for plugin in manager.plugins.values_mut() {
            plugin.stop_daemon().unwrap();
        }
    }

    #[test]
    fn plugin_replacement_supersedes_pending_config_reloads() {
        let _env_lock = crate::test_support::env_lock().blocking_lock();
        let _runtime_cache_lock = crate::test_support::runtime_cache_lock().blocking_lock();
        let root = tempfile::TempDir::new().unwrap();
        let _guard = crate::paths::push_test_path_root(root.path());
        let plugins_dir = crate::paths::plugins_dir().unwrap();
        let mut manager = PluginManager::new();
        let _pid_before = spawn_fixture_plugin(&mut manager, &plugins_dir, PLUGIN_ID);

        let (ticket, _receipt) = begin_and_record_reload(&mut manager, PLUGIN_ID, 1);
        manager.reload_plugin(PLUGIN_ID).unwrap();

        assert!(!manager.has_pending_config_reload(PLUGIN_ID));
        assert_eq!(
            manager.complete_config_reload(&ticket, ReloadDeliveryOutcome::Failed),
            CompletionDecision::Superseded
        );
        assert_eq!(
            manager.complete_config_reload(&ticket, ReloadDeliveryOutcome::Handled),
            CompletionDecision::Superseded
        );

        manager
            .plugins
            .get_mut(PLUGIN_ID)
            .unwrap()
            .stop_daemon()
            .unwrap();
    }

    #[test]
    fn full_profile_invalidation_supersedes_pending_config_reloads() {
        let _env_lock = crate::test_support::env_lock().blocking_lock();
        let _runtime_cache_lock = crate::test_support::runtime_cache_lock().blocking_lock();
        let root = tempfile::TempDir::new().unwrap();
        let _guard = crate::paths::push_test_path_root(root.path());
        let plugins_dir = crate::paths::plugins_dir().unwrap();
        let mut manager = PluginManager::new();
        let _pid_before = spawn_fixture_plugin(&mut manager, &plugins_dir, PLUGIN_ID);

        let (ticket, _receipt) = begin_and_record_reload(&mut manager, PLUGIN_ID, 1);
        {
            let _all = crate::plugins::config::profile_config_write_guard();
        }
        assert!(manager.reconcile_profile_generation().unwrap());

        assert!(!manager.has_pending_config_reload(PLUGIN_ID));
        assert_eq!(
            manager.complete_config_reload(&ticket, ReloadDeliveryOutcome::Handled),
            CompletionDecision::Superseded
        );

        for plugin in manager.plugins.values_mut() {
            plugin.stop_daemon().unwrap();
        }
    }
}
