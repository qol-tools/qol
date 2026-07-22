pub mod action_executor;
pub mod action_transport;
pub mod capabilities;
pub mod config;
pub mod daemon_health;
mod daemon_lifecycle;
pub mod daemon_supervisor;
pub mod daemon_tracker;
mod execution_contract;
#[cfg(test)]
mod execution_contract_tests;
pub mod identity_index;
pub mod loader;
pub mod manager;
pub mod manifest;
pub(crate) mod paths;
pub mod registry;
pub mod resolver;

pub use config::PluginConfigManager;
pub use daemon_lifecycle::lifeline_handoff;
pub use identity_index::{PluginDisplay, PluginIdentityIndex};
pub use loader::PluginLoader;
pub use manager::PluginManager;
pub use manifest::{ActionType, MenuItem, PluginId, PluginManifest, PluginUid};
pub use qol_conventions::is_reserved_plugin_id;
pub use resolver::PluginSource;

use anyhow::Result;
use std::path::PathBuf;
use std::process::Child;

use daemon_lifecycle::DaemonListener;
pub(crate) use execution_contract::{
    resolve_plugin_command_path_for_source, validate_execution_contract,
    validate_execution_contract_for_source,
};

#[derive(Debug)]
pub struct Plugin {
    pub id: PluginId,
    pub manifest: PluginManifest,
    pub path: PathBuf,
    pub source: PluginSource,
    daemon_process: Option<Child>,
    daemon_listener: Option<DaemonListener>,
}

impl Plugin {
    pub fn new(id: PluginId, manifest: PluginManifest, path: PathBuf) -> Self {
        Self::new_with_source(id, manifest, path, PluginSource::Installed)
    }

    pub fn new_with_source(
        id: PluginId,
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
            daemon_listener: None,
        }
    }

    pub fn uid(&self) -> PluginUid {
        match &self.manifest.plugin.uid {
            Some(uid) => uid.clone(),
            None => PluginUid::new(self.id.as_str()),
        }
    }

    pub fn start_daemon(&mut self) -> Result<()> {
        daemon_lifecycle::start_daemon(self)
    }

    pub fn daemon_pid(&self) -> Option<u32> {
        self.daemon_process.as_ref().map(|child| child.id())
    }

    pub fn reap_daemon_if_exited(&mut self) {
        daemon_lifecycle::reap_daemon_if_exited(self)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::manifest::{MenuConfig, PluginInfo};

    fn minimal_plugin(id: &str, uid: Option<&str>) -> Plugin {
        let plugin_info = PluginInfo {
            id: Some(PluginId::new(id)),
            uid: uid.map(PluginUid::new),
            name: "Test Plugin".to_string(),
            description: String::new(),
            version: "1.0.0".to_string(),
            author: None,
            platforms: None,
        };
        let manifest = PluginManifest {
            manifest_version: 1,
            plugin: plugin_info,
            menu: MenuConfig {
                label: String::new(),
                icon: None,
                items: vec![],
            },
            daemon: None,
            dependencies: None,
            runtime: None,
            actions: Default::default(),
            capabilities: Default::default(),
            build: Default::default(),
            traits: None,
            config: Default::default(),
            shortcuts: vec![],
        };
        Plugin::new(PluginId::new(id), manifest, std::path::PathBuf::new())
    }

    #[test]
    fn uid_returns_manifest_uid_when_present() {
        let plugin = minimal_plugin("plugin-foo", Some("uid-abc123"));
        assert_eq!(plugin.uid().as_str(), "uid-abc123");
    }

    #[test]
    fn uid_falls_back_to_id_when_manifest_uid_absent() {
        let plugin = minimal_plugin("plugin-foo", None);
        assert_eq!(plugin.uid().as_str(), "plugin-foo");
    }
}
