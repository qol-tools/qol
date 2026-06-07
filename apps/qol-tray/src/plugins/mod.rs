pub mod action_executor;
pub mod action_transport;
pub mod capabilities;
pub mod config;
mod daemon_lifecycle;
pub mod daemon_supervisor;
pub mod daemon_tracker;
mod execution_contract;
#[cfg(test)]
mod execution_contract_tests;
pub mod loader;
pub mod manager;
pub mod manifest;
pub(crate) mod paths;
pub mod registry;
mod reserved;
pub mod resolver;

pub use config::PluginConfigManager;
pub use loader::PluginLoader;
pub use manager::PluginManager;
pub use manifest::{ActionType, MenuItem, PluginManifest};
pub use reserved::is_reserved_plugin_id;
pub use resolver::PluginSource;

use anyhow::Result;
use std::path::PathBuf;
use std::process::Child;

pub(crate) use execution_contract::{
    resolve_plugin_command_path_for_source, validate_execution_contract,
    validate_execution_contract_for_source,
};

#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize)]
#[serde(transparent)]
pub struct PluginId(String);

impl PluginId {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for PluginId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl AsRef<str> for PluginId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl std::borrow::Borrow<str> for PluginId {
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl From<String> for PluginId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&str> for PluginId {
    fn from(s: &str) -> Self {
        Self(s.to_owned())
    }
}

impl<'de> serde::Deserialize<'de> for PluginId {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = PluginId;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a plugin id string")
            }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<PluginId, E> {
                Ok(PluginId(v.to_owned()))
            }
            fn visit_string<E: serde::de::Error>(self, v: String) -> Result<PluginId, E> {
                Ok(PluginId(v))
            }
        }
        d.deserialize_string(Visitor)
    }
}

#[derive(Debug)]
pub struct Plugin {
    pub id: PluginId,
    pub manifest: PluginManifest,
    pub path: PathBuf,
    pub source: PluginSource,
    daemon_process: Option<Child>,
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
