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

    pub fn ensure_plugin_daemon_running(&mut self, plugin_id: &str) -> Result<()> {
        runtime::ensure_plugin_daemon_running(self, plugin_id)
    }

    pub fn reap_exited_daemons(&mut self) {
        runtime::reap_exited_daemons(self)
    }
}

impl Default for PluginManager {
    fn default() -> Self {
        Self::new()
    }
}
