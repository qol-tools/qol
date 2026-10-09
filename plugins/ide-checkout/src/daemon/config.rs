use qol_config::{PluginConfigInspection, PluginConfigInspectionError};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

const CONFIG_CONTRACT: &str = qol_config::plugin_config_contract!();
const PLUGIN_ID: &str = env!("QOL_PLUGIN_ID");
const DEFAULT_CHECKOUT_DIR: &str = "qol-ide-checkout";

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

    pub fn checkout_root(&self) -> PathBuf {
        checkout_root_in(&self.temp_dir, &std::env::temp_dir())
    }

    #[cfg(test)]
    pub(crate) fn defaults() -> Self {
        qol_config::typed_defaults_from_contract(CONFIG_CONTRACT)
            .expect("config contract defaults must parse")
    }
}

fn checkout_root_in(configured: &std::path::Path, system_temp: &std::path::Path) -> PathBuf {
    if configured.as_os_str().is_empty() {
        return system_temp.join(DEFAULT_CHECKOUT_DIR);
    }
    configured.to_path_buf()
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
        assert_eq!(config.temp_dir, PathBuf::new());
        assert_eq!(
            config.checkout_root(),
            std::env::temp_dir().join("qol-ide-checkout")
        );
    }

    #[test]
    fn checkout_root_defaults_to_the_system_temp_directory() {
        let system_temp = PathBuf::from("/var/tmp");
        let cases = [
            ("", "/var/tmp/qol-ide-checkout"),
            ("/srv/checkouts", "/srv/checkouts"),
            ("relative", "relative"),
        ];
        for (configured, expected) in cases {
            assert_eq!(
                checkout_root_in(&PathBuf::from(configured), &system_temp),
                PathBuf::from(expected),
                "configured={configured:?}"
            );
        }
    }

    #[test]
    fn contract_defaults_match_runtime_type() {
        qol_config::validate_contract_defaults_match_type::<Config>(CONFIG_CONTRACT).unwrap();
    }
}
