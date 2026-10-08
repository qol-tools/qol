use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::account::ClaudeAccountSpec;

#[derive(Default, Deserialize)]
pub(crate) struct LaunchConfigFile {
    pub(crate) spawn_surface: Option<String>,
    pub(crate) spawn_cap: Option<bool>,
    pub(crate) spawn_cpu_weight: Option<u32>,
    pub(crate) spawn_io_weight: Option<u32>,
    pub(crate) spawn_cpu_quota: Option<String>,
    #[serde(default)]
    pub(crate) claude_accounts: BTreeMap<String, ClaudeAccountSpec>,
}

pub(crate) fn primary_config_path() -> Option<PathBuf> {
    qol_config::config_dir().map(|dir| dir.join("sessions.toml"))
}

pub fn sessions_config_path() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    candidates.extend(primary_config_path());
    if let Some(home) = dirs::home_dir() {
        candidates.push(
            home.join(".config")
                .join(qol_config::NAMESPACE)
                .join("sessions.toml"),
        );
    }
    candidates.into_iter().find(|path| path.exists())
}

pub(crate) fn read(path: &Path, what: &str) -> Result<Option<LaunchConfigFile>> {
    let encoded = match fs::read_to_string(path) {
        Ok(encoded) => encoded,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).with_context(|| format!("failed to read {what}")),
    };
    toml::from_str(&encoded)
        .map(Some)
        .with_context(|| format!("failed to parse {}", path.display()))
}
