pub mod contract;
pub mod defaults;
pub mod normalized;
pub mod validation;

pub use defaults::{
    defaults_json_from_contract, defaults_json_from_spec, deserialize_with_contract_defaults,
    typed_defaults_from_contract, typed_defaults_from_spec, validate_contract_defaults_match_type,
    validate_defaults_match_type,
};

use qol_conventions::ENV_PLUGIN_ID;
use serde::de::DeserializeOwned;
use std::fs;
use std::path::{Path, PathBuf};

pub const NAMESPACE: &str = "qol-tray";

pub const ACTIVE_INSTALL_ID_FILE: &str = "active-install-id";

#[macro_export]
macro_rules! plugin_config_contract {
    () => {
        include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/qol-config.toml"))
    };
}

fn resolve_namespaced(base: Option<PathBuf>) -> Option<PathBuf> {
    base.map(|path| path.join(NAMESPACE))
}

pub fn data_dir() -> Option<PathBuf> {
    resolve_namespaced(dirs::data_local_dir().or_else(dirs::data_dir))
}

pub fn config_dir() -> Option<PathBuf> {
    resolve_namespaced(dirs::config_dir())
}

pub fn data_subdir(name: &str) -> Option<PathBuf> {
    data_dir().map(|path| path.join(name))
}

#[doc(hidden)]
pub fn base_data_dir() -> Option<PathBuf> {
    data_dir()
}

pub fn config_roots() -> Vec<PathBuf> {
    let Some(base) = base_data_dir() else {
        return Vec::new();
    };
    let active_install_id = install_id_from_active_file(&base);
    assemble_config_roots(
        base,
        install_id_from_env(),
        active_install_id,
        dirs::config_dir().map(|p| p.join("qol-tray")),
    )
}

fn assemble_config_roots(
    base: PathBuf,
    install_id: Option<String>,
    active_install_id: Option<String>,
    user_config_dir: Option<PathBuf>,
) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(id) = install_id {
        roots.push(base.join("installs").join(id));
    }
    if let Some(id) = active_install_id {
        let candidate = base.join("installs").join(id);
        if !roots.contains(&candidate) {
            roots.push(candidate);
        }
    }
    if !roots.contains(&base) {
        roots.push(base);
    }
    if let Some(config_dir) = user_config_dir {
        if !roots.contains(&config_dir) {
            roots.push(config_dir);
        }
    }
    roots
}

pub fn plugin_config_paths(names: &[&str]) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    for root in config_roots() {
        for name in names {
            let candidate = root.join("plugins").join(name).join("config.json");
            if !paths.contains(&candidate) {
                paths.push(candidate);
            }
        }
    }
    paths
}

/// Load a plugin's config using the identity the host injects at launch.
///
/// The host sets [`qol_conventions::ENV_PLUGIN_ID`] on every plugin process it
/// spawns, so a plugin never has to hardcode its own id. `fallback_id` is used only when the
/// plugin is run standalone, outside the host (no env var). A present-but-invalid
/// injected id is a host bug and aborts loudly rather than silently loading
/// defaults.
pub fn load_plugin_config_from_env<T: DeserializeOwned + Default>(fallback_id: &str) -> T {
    let id = plugin_id_from_env(fallback_id);
    load_plugin_config(&[id.as_str()])
}

pub fn load_plugin_config_from_env_with_contract<T: DeserializeOwned>(
    fallback_id: &str,
    contract: &str,
) -> T {
    let id = plugin_id_from_env(fallback_id);
    load_plugin_config_with_contract(&[id.as_str()], contract)
}

/// Config file paths for the host-injected (or standalone-fallback) plugin id.
pub fn plugin_config_paths_from_env(fallback_id: &str) -> Vec<PathBuf> {
    let id = plugin_id_from_env(fallback_id);
    plugin_config_paths(&[id.as_str()])
}

/// The canonical plugin id the host injected at launch, or `fallback_id` when
/// run standalone. Aborts loudly on a present-but-invalid injected id.
pub fn plugin_id_from_env(fallback_id: &str) -> String {
    match std::env::var(ENV_PLUGIN_ID) {
        Ok(value) => {
            let trimmed = value.trim();
            assert!(
                valid_install_id(trimmed),
                "{ENV_PLUGIN_ID} {value:?} injected by the host is not a valid plugin id"
            );
            trimmed.to_string()
        }
        Err(_) => {
            assert!(
                valid_install_id(fallback_id),
                "standalone fallback plugin id {fallback_id:?} is not a valid plugin id"
            );
            fallback_id.to_string()
        }
    }
}

pub fn load_plugin_config<T: DeserializeOwned + Default>(names: &[&str]) -> T {
    load_plugin_config_or(names, T::default)
}

pub fn load_plugin_config_or<T: DeserializeOwned>(
    names: &[&str],
    default: impl FnOnce() -> T,
) -> T {
    for path in plugin_config_paths(names) {
        let contents = match fs::read_to_string(&path) {
            Ok(contents) => contents,
            Err(_) => continue,
        };
        let value = match serde_json::from_str::<serde_json::Value>(&contents) {
            Ok(value) => value,
            Err(error) => {
                eprintln!("[config] failed to parse {}: {}", path.display(), error);
                continue;
            }
        };
        match serde_json::from_value::<T>(canonicalize_whole_floats(value)) {
            Ok(config) => {
                eprintln!("[config] loaded from {}", path.display());
                return config;
            }
            Err(e) => {
                eprintln!("[config] failed to parse {}: {}", path.display(), e);
            }
        }
    }
    default()
}

fn canonicalize_whole_floats(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Number(number) => serde_json::Value::Number(canonical_number(number)),
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.into_iter().map(canonicalize_whole_floats).collect())
        }
        serde_json::Value::Object(map) => serde_json::Value::Object(
            map.into_iter()
                .map(|(key, item)| (key, canonicalize_whole_floats(item)))
                .collect(),
        ),
        other => other,
    }
}

fn canonical_number(number: serde_json::Number) -> serde_json::Number {
    if !number.is_f64() {
        return number;
    }
    let Some(value) = number.as_f64() else {
        return number;
    };
    let whole = value.fract() == 0.0 && value >= i64::MIN as f64 && value <= i64::MAX as f64;
    if whole {
        serde_json::Number::from(value as i64)
    } else {
        number
    }
}

pub fn load_plugin_config_with_contract<T: DeserializeOwned>(names: &[&str], contract: &str) -> T {
    let defaults = defaults_json_from_contract(contract).expect("config contract must validate");
    load_plugin_config_with_defaults(names, defaults)
}

fn load_plugin_config_with_defaults<T: DeserializeOwned>(
    names: &[&str],
    defaults: serde_json::Value,
) -> T {
    for path in plugin_config_paths(names) {
        let contents = match fs::read_to_string(&path) {
            Ok(contents) => contents,
            Err(_) => continue,
        };
        let overrides = match serde_json::from_str::<serde_json::Value>(&contents) {
            Ok(value) => value,
            Err(error) => {
                eprintln!("[config] failed to parse {}: {}", path.display(), error);
                continue;
            }
        };
        let merged =
            canonicalize_whole_floats(defaults::merge_json_defaults(defaults.clone(), overrides));
        match serde_json::from_value::<T>(merged) {
            Ok(config) => {
                eprintln!("[config] loaded from {}", path.display());
                return config;
            }
            Err(error) => {
                eprintln!("[config] failed to parse {}: {}", path.display(), error);
            }
        }
    }
    serde_json::from_value(defaults).expect("config contract defaults must deserialize")
}

pub fn install_id_from_env() -> Option<String> {
    let value = std::env::var(qol_conventions::ENV_INSTALL_ID).ok()?;
    let trimmed = value.trim();
    if !valid_install_id(trimmed) {
        return None;
    }
    Some(trimmed.to_string())
}

pub fn install_id_from_active_file(base_data_dir: &Path) -> Option<String> {
    let content = fs::read_to_string(base_data_dir.join(ACTIVE_INSTALL_ID_FILE)).ok()?;
    let trimmed = content.trim();
    if !valid_install_id(trimmed) {
        return None;
    }
    Some(trimmed.to_string())
}

pub fn valid_install_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whole_floats_canonicalize_to_integers_for_typed_configs() {
        #[derive(serde::Deserialize, Debug, PartialEq)]
        struct Typed {
            max_columns: usize,
            scale: f64,
            offset: i32,
        }
        let raw = serde_json::json!({
            "max_columns": 6.0,
            "scale": 1.5,
            "offset": -4.0,
            "extra": { "framerate": 60.0, "huge": 1.0e300 }
        });

        let canonical = canonicalize_whole_floats(raw);

        assert_eq!(
            canonical,
            serde_json::json!({
                "max_columns": 6,
                "scale": 1.5,
                "offset": -4,
                "extra": { "framerate": 60, "huge": 1.0e300 }
            })
        );
        assert_eq!(
            serde_json::from_value::<Typed>(canonical.clone()).unwrap(),
            Typed {
                max_columns: 6,
                scale: 1.5,
                offset: -4,
            }
        );
    }

    #[test]
    fn namespaced_resolver_joins_base_with_namespace() {
        let cases = [
            (
                Some(PathBuf::from("/data")),
                Some(PathBuf::from("/data/qol-tray")),
            ),
            (
                Some(PathBuf::from("/home/user/.local/share")),
                Some(PathBuf::from("/home/user/.local/share/qol-tray")),
            ),
            (None, None),
        ];
        for (base, expected) in cases {
            assert_eq!(resolve_namespaced(base.clone()), expected, "base: {base:?}");
        }
    }

    #[test]
    fn data_subdir_appends_under_namespaced_data_dir() {
        let Some(data) = data_dir() else {
            return;
        };
        assert_eq!(data_subdir("emu"), Some(data.join("emu")));
    }

    #[test]
    fn base_data_dir_is_an_alias_of_data_dir() {
        assert_eq!(base_data_dir(), data_dir());
        assert_eq!(NAMESPACE, "qol-tray");
    }

    #[test]
    fn without_pinned_install_base_is_the_only_data_root() {
        let base = PathBuf::from("/data/qol-tray");
        assert_eq!(
            assemble_config_roots(base.clone(), None, None, None),
            vec![base]
        );
    }

    #[test]
    fn explicit_install_env_is_searched_before_base() {
        let base = PathBuf::from("/data/qol-tray");
        let user_cfg = PathBuf::from("/home/user/.config/qol-tray");
        let roots = assemble_config_roots(
            base.clone(),
            Some("install-123".into()),
            None,
            Some(user_cfg.clone()),
        );
        assert_eq!(
            roots,
            vec![base.join("installs").join("install-123"), base, user_cfg]
        );
    }

    #[test]
    fn active_install_file_id_is_searched_after_env_and_deduped() {
        let base = PathBuf::from("/data/qol-tray");
        assert_eq!(
            assemble_config_roots(
                base.clone(),
                Some("env-id".into()),
                Some("active-id".into()),
                None,
            ),
            vec![
                base.join("installs").join("env-id"),
                base.join("installs").join("active-id"),
                base.clone(),
            ]
        );
        assert_eq!(
            assemble_config_roots(
                base.clone(),
                Some("same-id".into()),
                Some("same-id".into()),
                None,
            ),
            vec![base.join("installs").join("same-id"), base]
        );
    }

    #[test]
    fn active_install_file_id_searched_when_env_absent() {
        let base = PathBuf::from("/data/qol-tray");
        assert_eq!(
            assemble_config_roots(base.clone(), None, Some("active-id".into()), None),
            vec![base.join("installs").join("active-id"), base]
        );
    }

    #[test]
    fn no_installs_dir_resolved_without_explicit_env() {
        let base = PathBuf::from("/data/qol-tray");
        let roots = assemble_config_roots(base.clone(), None, None, None);
        assert!(roots
            .iter()
            .all(|root| !root.to_string_lossy().contains("installs")));
        assert_eq!(roots.first(), Some(&base));
    }

    #[test]
    fn user_config_dir_equal_to_base_is_deduped() {
        let base = PathBuf::from("/data/qol-tray");
        assert_eq!(
            assemble_config_roots(base.clone(), None, None, Some(base.clone())),
            vec![base]
        );
    }

    #[test]
    fn deserialize_with_contract_defaults_recursively_preserves_missing_values() {
        #[derive(Debug, serde::Deserialize, PartialEq)]
        struct Config {
            audio: Audio,
            video: Video,
        }
        #[derive(Debug, serde::Deserialize, PartialEq)]
        struct Audio {
            enabled: bool,
            inputs: Vec<String>,
            mic_device: String,
        }
        #[derive(Debug, serde::Deserialize, PartialEq)]
        struct Video {
            framerate: u32,
        }

        let contract = r#"
schema_version = 1

[field.audio_enabled]
type = "boolean"
config_key = "audio.enabled"
default = true

[field.audio_inputs]
type = "string_array"
config_key = "audio.inputs"
default = ["mic"]

[field.audio_mic_device]
type = "string"
config_key = "audio.mic_device"
default = "default"

[field.video_framerate]
type = "number"
config_key = "video.framerate"
default = 60
"#;

        let config: Config = deserialize_with_contract_defaults(
            contract,
            serde_json::json!({
                "audio": {
                    "enabled": false
                }
            }),
        )
        .unwrap();

        assert_eq!(
            config,
            Config {
                audio: Audio {
                    enabled: false,
                    inputs: vec!["mic".to_string()],
                    mic_device: "default".to_string(),
                },
                video: Video { framerate: 60 },
            }
        );
    }
}
