mod search_paths;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DevConfig {
    #[serde(default)]
    pub search_paths: Vec<PathBuf>,
}

impl DevConfig {
    pub fn load() -> Result<Self> {
        let path = crate::paths::dev_config_path()?;
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = std::fs::read_to_string(&path)?;
        let config: Self = serde_json::from_str(&content)?;
        Ok(config)
    }

    pub fn effective_search_paths(&self) -> Vec<PathBuf> {
        search_paths::effective_search_paths(self)
    }
}
