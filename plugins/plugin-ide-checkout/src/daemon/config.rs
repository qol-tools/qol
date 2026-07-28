use qol_config::{PluginConfigInspection, PluginConfigInspectionError};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

const CONFIG_CONTRACT: &str = qol_config::plugin_config_contract!();
const PLUGIN_ID: &str = env!("QOL_PLUGIN_ID");

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AppConfig {
    #[serde(default)]
    pub paths: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Config {
    #[serde(default)]
    pub apps: BTreeMap<String, AppConfig>,
    #[serde(rename = "tempDir")]
    pub temp_dir: PathBuf,
}

impl Config {
    pub fn load() -> Self {
        qol_config::load_plugin_config_from_env_with_contract(PLUGIN_ID, CONFIG_CONTRACT)
    }

    #[cfg(test)]
    pub(crate) fn defaults() -> Self {
        qol_config::typed_defaults_from_contract(CONFIG_CONTRACT)
            .expect("config contract defaults must parse")
    }
}

pub(crate) fn inspect() -> Result<PluginConfigInspection<Config>, PluginConfigInspectionError> {
    qol_config::inspect_plugin_config_from_env_with_contract(PLUGIN_ID, CONFIG_CONTRACT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_include_the_built_in_ides() {
        let config = Config::defaults();
        for id in ["idea", "vscode", "cursor", "zed"] {
            assert!(config.apps.contains_key(id), "missing default app {id}");
        }
        assert_eq!(config.temp_dir, PathBuf::from("/tmp/task-runner"));
    }

    #[test]
    fn contract_defaults_match_runtime_type() {
        qol_config::validate_contract_defaults_match_type::<Config>(CONFIG_CONTRACT).unwrap();
    }
}
