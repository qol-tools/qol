mod autostart;
mod lifeline_facade;
mod loading;
mod runtime;

use super::{Plugin, PluginId, PluginIdentityIndex};
use crate::plugins::action_executor::ProcessTracker;
use crate::plugins::action_transport::DaemonActionDispatch;
use crate::plugins::resolver::ResolutionReport;
use anyhow::Result;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

pub struct PluginManager {
    plugins: HashMap<PluginId, Plugin>,
    pub(super) identity_index: PluginIdentityIndex,
    resolution_report: ResolutionReport,
    last_profile_generation: u64,
    last_reconciled_plugin_generations: HashMap<String, u64>,
    lifecycle_cancellation: Arc<qol_process::CancellationToken>,
    process_tracker: Arc<ProcessTracker>,
    #[cfg(test)]
    profile_reconciliation_count: u64,
}

impl PluginManager {
    pub fn new() -> Self {
        Self {
            plugins: HashMap::new(),
            identity_index: PluginIdentityIndex::default(),
            resolution_report: ResolutionReport::default(),
            last_profile_generation: crate::plugins::config::current_profile_config_generation(),
            last_reconciled_plugin_generations: HashMap::new(),
            lifecycle_cancellation: Arc::new(qol_process::CancellationToken::new()),
            process_tracker: Arc::new(ProcessTracker::default()),
            #[cfg(test)]
            profile_reconciliation_count: 0,
        }
    }

    pub(crate) fn process_tracker(&self) -> Arc<ProcessTracker> {
        Arc::clone(&self.process_tracker)
    }

    pub fn load_plugins(&mut self) -> Result<()> {
        loading::load_plugins(self)?;
        self.last_profile_generation = crate::plugins::config::current_profile_config_generation();
        self.last_reconciled_plugin_generations.clear();
        Ok(())
    }

    pub fn autostart_daemons(&mut self) {
        if self.lifecycle_cancellation.is_cancelled() {
            return;
        }
        autostart::start_plugin_daemons(
            self.plugins.values_mut(),
            Some(&self.lifecycle_cancellation),
        );
    }

    pub fn reconcile_and_autostart_daemons(&mut self) {
        if self.lifecycle_cancellation.is_cancelled() {
            return;
        }
        let reconciled = match self.reconcile_profile_generation() {
            Ok(reconciled) => reconciled,
            Err(error) => {
                log::error!(
                    "Failed to reconcile profile generation before daemon autostart: {error:#}"
                );
                return;
            }
        };
        if !reconciled {
            self.autostart_daemons();
        }
    }

    pub fn wait_for_autostart_daemons_ready(&self, timeout: Duration) -> Vec<String> {
        autostart::wait_for_autostart_daemons_ready(self.plugins.values(), timeout)
    }

    pub fn reload_plugins(&mut self) -> Result<()> {
        if self.lifecycle_cancellation.is_cancelled() {
            return Ok(());
        }
        let observed_generation = crate::plugins::config::current_profile_config_generation();
        runtime::reload_plugins(self)?;
        self.last_profile_generation = observed_generation;
        self.last_reconciled_plugin_generations.clear();
        Ok(())
    }

    pub fn reload_plugin(&mut self, plugin_id: &str) -> Result<()> {
        if self.lifecycle_cancellation.is_cancelled() {
            return Ok(());
        }
        let consumed_generation = runtime::reload_plugin(self, plugin_id)?;
        self.acknowledge_profile_plugin_generation(plugin_id, consumed_generation);
        Ok(())
    }

    pub fn reload_plugins_if_changed(&mut self) -> Result<bool> {
        self.reconcile_profile_generation()
    }

    pub fn reconcile_profile_generation(&mut self) -> Result<bool> {
        if self.lifecycle_cancellation.is_cancelled() {
            return Ok(false);
        }
        let Some((observed_generation, invalidation)) =
            crate::plugins::config::profile_config_invalidation_since(self.last_profile_generation)
        else {
            return Ok(false);
        };
        #[cfg(test)]
        {
            self.profile_reconciliation_count += 1;
        }
        let reconciled = match invalidation {
            crate::plugins::config::ProfileConfigInvalidation::All => {
                runtime::reload_plugins(self)?;
                self.last_reconciled_plugin_generations.clear();
                true
            }
            crate::plugins::config::ProfileConfigInvalidation::Plugins(plugin_ids) => {
                let plugin_ids = self.resolve_profile_plugin_ids(plugin_ids);
                let mut reloaded = false;
                for plugin_id in &plugin_ids {
                    let invalidation_generation =
                        self.profile_plugin_invalidation_generation(plugin_id);
                    if self
                        .last_reconciled_plugin_generations
                        .get(plugin_id)
                        .is_some_and(|generation| *generation >= invalidation_generation)
                    {
                        continue;
                    }
                    self.reload_plugin(plugin_id)?;
                    reloaded = true;
                }
                reloaded
            }
        };
        self.last_profile_generation = observed_generation;
        Ok(reconciled)
    }

    fn resolve_profile_plugin_ids(&self, invalidated_ids: Vec<String>) -> Vec<String> {
        let invalidated_ids = invalidated_ids.into_iter().collect::<HashSet<_>>();
        let plugin_ids = self
            .plugins
            .values()
            .filter(|plugin| {
                invalidated_ids.contains(plugin.id.as_str())
                    || invalidated_ids.contains(plugin.uid().as_str())
            })
            .map(|plugin| plugin.id.to_string())
            .collect::<HashSet<_>>();
        let mut plugin_ids = plugin_ids.into_iter().collect::<Vec<_>>();
        plugin_ids.sort_unstable();
        plugin_ids
    }

    fn profile_plugin_invalidation_generation(&self, plugin_id: &str) -> u64 {
        let Some(plugin) = self.plugins.get(plugin_id) else {
            return crate::plugins::config::profile_config_plugin_generation(plugin_id);
        };
        crate::plugins::config::profile_config_plugin_generation(plugin.id.as_str()).max(
            crate::plugins::config::profile_config_plugin_generation(plugin.uid().as_str()),
        )
    }

    pub fn acknowledge_profile_plugin_generation(&mut self, plugin_id: &str, generation: u64) {
        self.last_reconciled_plugin_generations
            .insert(plugin_id.to_string(), generation);
        qol_runtime::probe!(
            "PLUGIN_RELOAD",
            "plugin={plugin_id} stage=ack scope=single consumed_generation={generation} acknowledged_generation={generation}"
        );
    }

    #[cfg(test)]
    pub(crate) fn profile_reconciliation_count(&self) -> u64 {
        self.profile_reconciliation_count
    }

    #[cfg(test)]
    pub(crate) fn insert_plugin_for_test(&mut self, plugin: Plugin) {
        self.plugins.insert(plugin.id.clone(), plugin);
    }

    pub fn shutdown(&mut self) {
        self.lifecycle_cancellation.cancel();
        runtime::shutdown(self);
    }

    pub fn lifecycle_cancellation(&self) -> Arc<qol_process::CancellationToken> {
        Arc::clone(&self.lifecycle_cancellation)
    }

    pub fn get(&self, plugin_id: &str) -> Option<&Plugin> {
        self.plugins.get(plugin_id)
    }

    pub fn plugins(&self) -> impl Iterator<Item = &Plugin> {
        self.plugins.values()
    }

    pub fn identity_index(&self) -> &PluginIdentityIndex {
        &self.identity_index
    }

    pub fn last_resolution_report(&self) -> &ResolutionReport {
        &self.resolution_report
    }

    pub(super) fn set_resolution_report(&mut self, report: ResolutionReport) {
        self.resolution_report = report;
    }

    pub fn restart_running_plugin_daemon(&mut self, plugin_id: &str) -> Result<()> {
        if self.lifecycle_cancellation.is_cancelled() {
            anyhow::bail!("plugin daemon lifecycle is shutting down")
        }
        runtime::restart_running_plugin_daemon(self, plugin_id)
    }

    pub fn restart_running_gpui_daemons(&mut self) -> Vec<PluginId> {
        let plugin_ids = self.running_gpui_daemon_ids();
        let mut restarted = Vec::new();
        for plugin_id in plugin_ids {
            match self.restart_running_plugin_daemon(plugin_id.as_str()) {
                Ok(()) => restarted.push(plugin_id),
                Err(error) => {
                    log::error!(
                        "Failed to restart GPUI daemon {} after theme accent change: {}",
                        plugin_id,
                        error
                    );
                }
            }
        }
        restarted
    }

    pub fn broadcast_theme_to_running_gpui_daemons(
        &mut self,
        native: &str,
        accent: &str,
    ) -> Vec<PluginId> {
        let plugin_ids = self.running_gpui_daemon_ids();
        let mut reached = Vec::new();
        for plugin_id in plugin_ids {
            let socket = self
                .plugins
                .get(&plugin_id)
                .and_then(crate::plugins::action_executor::daemon_socket);
            let Some(socket) = socket else {
                log::warn!(
                    "GPUI daemon {} has no socket, falling back to a restart for the theme broadcast",
                    plugin_id
                );
                match self.restart_running_plugin_daemon(plugin_id.as_str()) {
                    Ok(()) => reached.push(plugin_id),
                    Err(error) => {
                        log::error!(
                            "Failed to restart GPUI daemon {} after theme change: {}",
                            plugin_id,
                            error
                        );
                    }
                }
                continue;
            };
            let dispatch =
                crate::plugins::action_transport::dispatch_daemon_theme(&socket, native, accent);
            if matches!(dispatch, DaemonActionDispatch::Handled { .. }) {
                reached.push(plugin_id);
                continue;
            }
            log::warn!(
                "GPUI daemon {} did not handle the theme broadcast (dispatch: {dispatch:?}), falling back to a restart",
                plugin_id
            );
            match self.restart_running_plugin_daemon(plugin_id.as_str()) {
                Ok(()) => reached.push(plugin_id),
                Err(error) => {
                    log::error!(
                        "Failed to restart GPUI daemon {} after theme change: {}",
                        plugin_id,
                        error
                    );
                }
            }
        }
        reached
    }

    pub fn running_gpui_daemon_ids(&self) -> Vec<PluginId> {
        self.plugins
            .values()
            .filter(|plugin| plugin.manifest.capabilities.gpui)
            .filter(|plugin| plugin.daemon_pid().is_some())
            .map(|plugin| plugin.id.clone())
            .collect()
    }

    pub fn ensure_plugin_daemon_running(&mut self, plugin_id: &str) -> Result<()> {
        if self.lifecycle_cancellation.is_cancelled() {
            anyhow::bail!("plugin daemon lifecycle is shutting down")
        }
        runtime::ensure_plugin_daemon_running(self, plugin_id)
    }

    pub fn reap_exited_daemons(&mut self) {
        runtime::reap_exited_daemons(self)
    }

    pub fn plugin_daemon_pid(&self, plugin_id: &PluginId) -> Option<u32> {
        self.plugins
            .get(plugin_id)
            .and_then(|plugin| plugin.daemon_pid())
    }

    pub fn daemon_health_snapshots(
        &self,
    ) -> Vec<(
        PluginId,
        crate::plugins::daemon_health::DaemonExpectation,
        Option<u32>,
    )> {
        self.plugins
            .values()
            .map(|plugin| {
                (
                    plugin.id.clone(),
                    autostart::daemon_expectation(plugin),
                    plugin.daemon_pid(),
                )
            })
            .collect()
    }
}

impl Default for PluginManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::manifest::PluginManifest;
    use crate::plugins::resolver::PluginSource;

    fn manifest(id: &str, daemon: bool) -> PluginManifest {
        let daemon_section = if daemon {
            "\n[daemon]\nenabled = true\ncommand = \"daemon.sh\"\n"
        } else {
            ""
        };
        toml::from_str(&format!(
            r#"
[plugin]
id = "{id}"
name = "{id}"
description = ""
version = "1.0.0"

[menu]
label = "{id}"
items = []
{daemon_section}"#
        ))
        .unwrap()
    }

    fn plugin_with_daemon(id: &str, gpui: bool, daemon: std::process::Child) -> Plugin {
        let mut plugin = Plugin::new(
            PluginId::new(id),
            manifest(id, true),
            std::path::PathBuf::new(),
        );
        plugin.manifest.capabilities.gpui = gpui;
        plugin.daemon_process = Some(daemon);
        plugin
    }

    fn insert_plugin(
        manager: &mut PluginManager,
        root: &std::path::Path,
        id: &str,
        daemon: bool,
        source: PluginSource,
        marker: bool,
    ) {
        let dir = root.join(id);
        std::fs::create_dir_all(&dir).unwrap();
        if marker {
            std::fs::write(dir.join(autostart::DEV_DAEMON_AUTOSTART_MARKER), "").unwrap();
        }
        manager.plugins.insert(
            PluginId::new(id),
            Plugin::new_with_source(PluginId::new(id), manifest(id, daemon), dir, source),
        );
    }

    #[test]
    fn daemon_health_snapshots_classify_by_autostart_policy() {
        use crate::plugins::daemon_health::DaemonExpectation;

        let root = tempfile::TempDir::new().unwrap();
        let mut manager = PluginManager::new();
        let cases = [
            (
                "plugin-installed",
                true,
                PluginSource::Installed,
                false,
                DaemonExpectation::Supervised,
            ),
            (
                "plugin-dev-blocked",
                true,
                PluginSource::DevLinked,
                false,
                DaemonExpectation::AutostartBlocked,
            ),
            (
                "plugin-dev-opted-in",
                true,
                PluginSource::DevLinked,
                true,
                DaemonExpectation::Supervised,
            ),
            (
                "plugin-no-daemon",
                false,
                PluginSource::Installed,
                false,
                DaemonExpectation::NotExpected,
            ),
        ];
        for (id, daemon, source, marker, _) in cases.clone() {
            insert_plugin(&mut manager, root.path(), id, daemon, source, marker);
        }

        let snapshots = manager.daemon_health_snapshots();
        for (id, _, _, _, expected) in cases {
            let (_, expectation, _) = snapshots
                .iter()
                .find(|(plugin_id, _, _)| plugin_id.as_str() == id)
                .unwrap_or_else(|| panic!("missing snapshot for {id}"));
            assert_eq!(*expectation, expected, "plugin: {id}");
        }
    }

    #[test]
    fn unchanged_generation_reconciliation_does_not_read_profile_state() {
        let _env = crate::test_support::env_lock().blocking_lock();
        let root = tempfile::TempDir::new().unwrap();
        let _path = crate::paths::push_test_path_root(root.path());
        let mut manager = PluginManager::new();

        crate::plugins::config::reset_profile_config_read_count();
        assert!(!manager.reconcile_profile_generation().unwrap());
        assert_eq!(crate::plugins::config::profile_config_read_count(), 0);
        assert_eq!(manager.profile_reconciliation_count(), 0);
    }

    #[test]
    fn profile_generation_event_reconciles_once() {
        let _env = crate::test_support::env_lock().blocking_lock();
        let root = tempfile::TempDir::new().unwrap();
        let _path = crate::paths::push_test_path_root(root.path());
        let mut manager = PluginManager::new();

        let _profile_guard = crate::plugins::config::profile_config_write_guard();
        crate::daemon::ConfigBus::new().config_changed(crate::daemon::ConfigKind::Profile);
        drop(_profile_guard);
        assert!(manager.reconcile_profile_generation().unwrap());
        assert!(!manager.reconcile_profile_generation().unwrap());
        assert_eq!(manager.profile_reconciliation_count(), 1);
    }

    #[test]
    fn changed_generation_reconciliation_does_not_hold_a_profile_read_guard() {
        let _env = crate::test_support::env_lock().blocking_lock();
        let root = tempfile::TempDir::new().unwrap();
        let _path = crate::paths::push_test_path_root(root.path());
        let mut manager = PluginManager::new();
        {
            let _profile_guard = crate::plugins::config::profile_config_write_guard();
        }

        crate::plugins::config::reset_profile_config_read_count();
        assert!(manager.reconcile_profile_generation().unwrap());
        assert_eq!(crate::plugins::config::profile_config_read_count(), 0);
    }

    #[cfg(unix)]
    #[test]
    fn running_gpui_daemon_ids_skips_headless_and_stopped_plugins() {
        let root = tempfile::tempdir().unwrap();
        let _guard = crate::paths::push_test_path_root(root.path());
        let mut manager = PluginManager::new();
        manager.plugins.insert(
            PluginId::new("plugin-gpui-running"),
            plugin_with_daemon(
                "plugin-gpui-running",
                true,
                std::process::Command::new("true").spawn().unwrap(),
            ),
        );
        manager.plugins.insert(
            PluginId::new("plugin-headless-running"),
            plugin_with_daemon(
                "plugin-headless-running",
                false,
                std::process::Command::new("true").spawn().unwrap(),
            ),
        );
        let mut stopped_gpui = Plugin::new(
            PluginId::new("plugin-gpui-stopped"),
            manifest("plugin-gpui-stopped", true),
            std::path::PathBuf::new(),
        );
        stopped_gpui.manifest.capabilities.gpui = true;
        manager
            .plugins
            .insert(PluginId::new("plugin-gpui-stopped"), stopped_gpui);

        let ids = manager.running_gpui_daemon_ids();

        assert_eq!(ids.len(), 1);
        assert_eq!(ids[0].as_str(), "plugin-gpui-running");

        manager.shutdown();
    }
}
