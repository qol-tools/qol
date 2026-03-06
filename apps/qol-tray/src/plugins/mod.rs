pub mod action_executor;
pub mod action_transport;
pub mod config;
mod daemon_lifecycle;
#[allow(dead_code)]
pub mod daemon_tracker;
mod execution_contract;
#[cfg(test)]
mod execution_contract_tests;
pub mod loader;
pub mod log_control;
pub mod manager;
pub mod manifest;
pub mod resolver;

pub use config::PluginConfigManager;
pub use loader::PluginLoader;
pub use manager::PluginManager;
pub use manifest::{ActionType, MenuItem, PluginManifest};
pub use resolver::PluginSource;

use anyhow::Result;
use std::path::PathBuf;
use std::process::Child;

pub(crate) use execution_contract::{
    resolve_plugin_command_path_for_source, validate_execution_contract,
    validate_execution_contract_for_source,
};

#[derive(Debug)]
pub struct Plugin {
    pub id: String,
    pub manifest: PluginManifest,
    pub path: PathBuf,
    pub source: PluginSource,
    daemon_process: Option<Child>,
}

impl Plugin {
    pub fn new(id: String, manifest: PluginManifest, path: PathBuf) -> Self {
        Self::new_with_source(id, manifest, path, PluginSource::Installed)
    }

    pub fn new_with_source(
        id: String,
        manifest: PluginManifest,
        path: PathBuf,
        source: PluginSource,
    ) -> Self {
        Self {
            id,
            manifest,
            path,
            source,
            daemon_process: None,
        }
    }

    pub fn start_daemon(&mut self) -> Result<()> {
        daemon_lifecycle::start_daemon(self)
    }

    pub fn daemon_pid(&self) -> Option<u32> {
        self.daemon_process.as_ref().map(|child| child.id())
    }

    pub fn stop_daemon(&mut self) -> Result<()> {
        daemon_lifecycle::stop_daemon(self)
    }
}

impl Drop for Plugin {
    fn drop(&mut self) {
        let _ = self.stop_daemon();
    }
}
