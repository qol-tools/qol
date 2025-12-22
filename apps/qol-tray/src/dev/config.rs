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
        let config: DevConfig = serde_json::from_str(&content)?;
        Ok(config)
    }

    pub fn effective_search_paths(&self) -> Vec<PathBuf> {
        let mut paths = Vec::new();

        if !self.search_paths.is_empty() {
            paths.extend(self.search_paths.clone());
        } else {
            if let Some(home) = dirs::home_dir() {
                for name in Self::common_dev_dirs() {
                    let path = home.join(name);
                    if path.is_dir() {
                        paths.push(path);
                    }
                }
            }

            if let Ok(cwd) = std::env::current_dir() {
                if let Some(parent) = cwd.parent() {
                    paths.push(parent.to_path_buf());
                }
            }
        }

        let mut unique_paths = Vec::new();
        let mut seen = std::collections::HashSet::new();

        for path in paths {
            let abs_path = path.canonicalize().unwrap_or(path);
            if seen.insert(abs_path.clone()) {
                unique_paths.push(abs_path);
            }
        }

        unique_paths
    }

    fn common_dev_dirs() -> &'static [&'static str] {
        &[
            "Developer",
            "Projects",
            "repos",
            "src",
            "code",
            "dev",
            "Git",
            "GitHub",
            "work",
            "workspace",
            "Documents/GitHub",
            "Documents/Projects",
        ]
    }
}
