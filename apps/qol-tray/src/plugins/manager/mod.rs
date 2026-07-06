mod autostart;
mod lifeline_facade;
mod loading;
mod runtime;

use super::{Plugin, PluginId, PluginIdentityIndex};
use crate::plugins::resolver::ResolutionReport;
use anyhow::Result;
use std::collections::HashMap;
use std::time::Duration;

pub struct PluginManager {
    plugins: HashMap<PluginId, Plugin>,
    pub(super) identity_index: PluginIdentityIndex,
    resolution_report: ResolutionReport,
    last_state_hash: Option<String>,
}

impl PluginManager {
    pub fn new() -> Self {
        Self {
            plugins: HashMap::new(),
            identity_index: PluginIdentityIndex::default(),
            resolution_report: ResolutionReport::default(),
            last_state_hash: None,
        }
    }

    pub fn load_plugins(&mut self) -> Result<()> {
        loading::load_plugins(self)?;
        self.last_state_hash = Some(runtime::hash_active_plugin_state());
        Ok(())
    }

    pub fn autostart_daemons(&mut self) {
        autostart::start_plugin_daemons(self.plugins.values_mut());
    }

    pub fn wait_for_autostart_daemons_ready(&self, timeout: Duration) -> Vec<String> {
        autostart::wait_for_autostart_daemons_ready(self.plugins.values(), timeout)
    }

    pub fn reload_plugins(&mut self) -> Result<()> {
        runtime::reload_plugins(self)?;
        self.last_state_hash = Some(runtime::hash_active_plugin_state());
        Ok(())
    }

    pub fn reload_plugins_if_changed(&mut self) -> Result<bool> {
        let current = runtime::hash_active_plugin_state();
        if self.last_state_hash.as_ref() == Some(&current) {
            return Ok(false);
        }
        self.reload_plugins()?;
        Ok(true)
    }

    pub fn shutdown(&mut self) {
        runtime::shutdown(self);
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

    pub fn running_gpui_daemon_ids(&self) -> Vec<PluginId> {
        self.plugins
            .values()
            .filter(|plugin| plugin.manifest.capabilities.gpui)
            .filter(|plugin| plugin.daemon_pid().is_some())
            .map(|plugin| plugin.id.clone())
            .collect()
    }

    pub fn ensure_plugin_daemon_running(&mut self, plugin_id: &str) -> Result<()> {
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

    #[cfg(unix)]
    #[test]
    fn running_gpui_daemon_ids_skips_headless_and_stopped_plugins() {
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
