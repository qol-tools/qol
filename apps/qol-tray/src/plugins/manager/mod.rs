mod autostart;
mod loading;
mod runtime;

use super::{Plugin, PluginId};
use crate::plugins::resolver::ResolutionReport;
use anyhow::Result;
use std::collections::HashMap;

pub struct PluginManager {
    plugins: HashMap<PluginId, Plugin>,
    resolution_report: ResolutionReport,
}

impl PluginManager {
    pub fn new() -> Self {
        Self {
            plugins: HashMap::new(),
            resolution_report: ResolutionReport::default(),
        }
    }

    pub fn load_plugins(&mut self) -> Result<()> {
        loading::load_plugins(self)
    }

    pub fn reload_plugins(&mut self) -> Result<()> {
        runtime::reload_plugins(self)
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
}

impl Default for PluginManager {
    fn default() -> Self {
        Self::new()
    }
}
