use crate::{
    canonicalize_whole_floats, defaults, defaults_json_from_contract, plugin_config_paths,
    plugin_id_from_env,
};
use serde::de::DeserializeOwned;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq)]
pub struct PluginConfigInspection<T> {
    pub config: T,
    pub source: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginConfigInspectionError {
    path: PathBuf,
    message: String,
}

impl PluginConfigInspectionError {
    pub fn path(&self) -> &Path {
        &self.path
    }

    fn new(path: &Path, message: impl Into<String>) -> Self {
        Self {
            path: path.to_path_buf(),
            message: message.into(),
        }
    }
}

impl fmt::Display for PluginConfigInspectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "failed to inspect {}: {}",
            self.path.display(),
            self.message
        )
    }
}

impl std::error::Error for PluginConfigInspectionError {}

pub fn inspect_plugin_config_from_env_with_contract<T: DeserializeOwned>(
    fallback_id: &str,
    contract: &str,
) -> Result<PluginConfigInspection<T>, PluginConfigInspectionError> {
    let id = plugin_id_from_env(fallback_id);
    inspect_paths_with_contract(&plugin_config_paths(&[id.as_str()]), contract)
}

fn inspect_paths_with_contract<T: DeserializeOwned>(
    paths: &[PathBuf],
    contract: &str,
) -> Result<PluginConfigInspection<T>, PluginConfigInspectionError> {
    let defaults = defaults_json_from_contract(contract).expect("config contract must validate");
    for path in paths {
        let contents = match fs::read_to_string(path) {
            Ok(contents) => contents,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(PluginConfigInspectionError::new(path, error.to_string()));
            }
        };
        let overrides = serde_json::from_str(&contents)
            .map_err(|error| PluginConfigInspectionError::new(path, error.to_string()))?;
        let merged =
            canonicalize_whole_floats(defaults::merge_json_defaults(defaults.clone(), overrides));
        let config = serde_json::from_value(merged)
            .map_err(|error| PluginConfigInspectionError::new(path, error.to_string()))?;
        return Ok(PluginConfigInspection {
            config,
            source: Some(path.clone()),
        });
    }
    Ok(PluginConfigInspection {
        config: serde_json::from_value(defaults)
            .expect("config contract defaults must deserialize"),
        source: None,
    })
}

#[cfg(test)]
mod tests {
    use super::inspect_paths_with_contract;
    use crate::parse_error_marker_path;
    use serde::Deserialize;
    use std::fs;

    const CONTRACT: &str = r#"
schema_version = 1

[field.count]
type = "number"
default = 1
"#;

    #[derive(Debug, Deserialize, PartialEq)]
    struct Config {
        count: usize,
    }

    #[test]
    fn inspection_never_changes_parse_markers() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("config.json");
        let marker = parse_error_marker_path(&path);
        fs::write(&path, "not json").unwrap();

        let error = inspect_paths_with_contract::<Config>(std::slice::from_ref(&path), CONTRACT)
            .unwrap_err();

        assert_eq!(error.path(), path);
        assert!(!marker.exists());

        fs::write(&path, "{\"count\": 3.0}").unwrap();
        fs::write(&marker, "previous diagnostic").unwrap();
        let inspected =
            inspect_paths_with_contract::<Config>(std::slice::from_ref(&path), CONTRACT).unwrap();

        assert_eq!(inspected.config, Config { count: 3 });
        assert_eq!(inspected.source.as_deref(), Some(path.as_path()));
        assert_eq!(fs::read_to_string(marker).unwrap(), "previous diagnostic");
    }

    #[test]
    fn missing_config_uses_contract_defaults() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("missing.json");

        let inspected =
            inspect_paths_with_contract::<Config>(std::slice::from_ref(&path), CONTRACT).unwrap();

        assert_eq!(inspected.config, Config { count: 1 });
        assert_eq!(inspected.source, None);
    }

    #[test]
    fn invalid_higher_precedence_config_is_reported() {
        let root = tempfile::tempdir().unwrap();
        let higher = root.path().join("higher.json");
        let fallback = root.path().join("fallback.json");
        fs::write(&higher, "{\"count\": false}").unwrap();
        fs::write(&fallback, "{\"count\": 4}").unwrap();

        let error = inspect_paths_with_contract::<Config>(&[higher.clone(), fallback], CONTRACT)
            .unwrap_err();

        assert_eq!(error.path(), higher);
    }
}
