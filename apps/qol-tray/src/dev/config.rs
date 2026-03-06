mod loading;
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
        loading::load()
    }

    pub fn effective_search_paths(&self) -> Vec<PathBuf> {
        search_paths::effective_search_paths(self)
    }
}
