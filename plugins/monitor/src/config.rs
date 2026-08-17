use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use qol_profile_sync::{device_local_dir, SyncPaths};

use crate::monitor::BrightnessPolicy;

pub const DEVICE_NAMESPACE: &str = "monitor";
pub const DEVICE_CONFIG_FILE: &str = "config.json";
pub const SESSION_SUBDIR: &str = "session";
const PREFERRED_MAX: u8 = 100;
const PLUGIN_ID: &str = env!("QOL_PLUGIN_ID");
const CONFIG_CONTRACT: &str = qol_config::plugin_config_contract!();

#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct DeviceConfig {
    pub preferred_brightness: BTreeMap<String, BrightnessPreference>,
    pub policy: BTreeMap<String, PolicySelection>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BrightnessPreference {
    pub brightness: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PolicySelection {
    pub policy: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigOrigin {
    HostStore,
    MaterializedFile,
    LegacyAdopted,
    Defaults,
}

impl ConfigOrigin {
    pub fn label(self) -> &'static str {
        match self {
            Self::HostStore => "host config store",
            Self::MaterializedFile => "materialized config file",
            Self::LegacyAdopted => "legacy device config",
            Self::Defaults => "defaults",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct LegacyDeviceConfig {
    #[serde(default)]
    preferred_brightness: BTreeMap<String, u8>,
    #[serde(default)]
    policy: BTreeMap<String, String>,
}

impl DeviceConfig {
    pub fn validated(mut self) -> Self {
        self.preferred_brightness
            .retain(|_, preference| preference.brightness <= PREFERRED_MAX);
        self.policy
            .retain(|_, selection| BrightnessPolicy::parse(&selection.policy).is_some());
        self
    }

    pub fn policy_for(&self, display_id: &str) -> BrightnessPolicy {
        self.policy
            .get(display_id)
            .and_then(|selection| BrightnessPolicy::parse(&selection.policy))
            .unwrap_or_default()
    }

    pub fn preferred_for(&self, display_id: &str) -> Option<u8> {
        self.preferred_brightness
            .get(display_id)
            .map(|preference| preference.brightness)
    }
}

pub fn config_root() -> Option<PathBuf> {
    qol_config::config_dir()
}

pub fn profile_root(config_root: &Path) -> PathBuf {
    config_root.join("profile")
}

pub fn active_profile_name(config_root: &Path) -> String {
    SyncPaths::new(profile_root(config_root)).active_profile_name()
}

pub fn device_dir(config_root: &Path) -> Result<PathBuf> {
    let profile = active_profile_name(config_root);
    device_local_dir(&profile, DEVICE_NAMESPACE)
        .with_context(|| format!("invalid active profile `{profile}`"))
        .map(|dir| profile_root(config_root).join(dir))
}

pub fn session_dir(config_root: &Path) -> Result<PathBuf> {
    device_dir(config_root).map(|dir| dir.join(SESSION_SUBDIR))
}

pub fn hotkeys_path(config_root: &Path) -> Result<PathBuf> {
    Ok(SyncPaths::new(profile_root(config_root)).hotkeys_path())
}

pub fn legacy_config_path(config_root: &Path) -> Result<PathBuf> {
    device_dir(config_root).map(|dir| dir.join(DEVICE_CONFIG_FILE))
}

pub fn materialized_config_path() -> Option<PathBuf> {
    qol_config::config_dir().map(|dir| dir.join("plugins").join(PLUGIN_ID).join(DEVICE_CONFIG_FILE))
}

pub fn load() -> Result<DeviceConfig> {
    Ok(load_with_origin(None).0)
}

pub fn load_with_origin(legacy_root: Option<&Path>) -> (DeviceConfig, ConfigOrigin) {
    load_from(
        qol_runtime::plugin_config::load_json(),
        materialized_config_path().as_deref(),
        legacy_root,
    )
}

fn load_from(
    store_json: Option<serde_json::Value>,
    materialized: Option<&Path>,
    legacy_root: Option<&Path>,
) -> (DeviceConfig, ConfigOrigin) {
    if let Some(json) = store_json.as_ref().filter(|value| !value.is_null()) {
        match parse_store(json) {
            Ok(config) => return (config, ConfigOrigin::HostStore),
            Err(error) => eprintln!("[plugin-monitor] host config store unreadable: {error:#}"),
        }
    }
    if let Some(path) = materialized {
        if let Ok(config) = load_materialized(path) {
            return (config, ConfigOrigin::MaterializedFile);
        }
    }
    if let Some(root) = legacy_root {
        if let Ok(config) = load_legacy(root) {
            let _ = qol_runtime::plugin_config::save(&config);
            return (config, ConfigOrigin::LegacyAdopted);
        }
    }
    (DeviceConfig::default(), ConfigOrigin::Defaults)
}

fn load_materialized(path: &Path) -> Result<DeviceConfig> {
    let content =
        std::fs::read(path).with_context(|| format!("failed to read config {}", path.display()))?;
    let json: serde_json::Value = serde_json::from_slice(&content)
        .with_context(|| format!("failed to parse config {}", path.display()))?;
    parse_store(&json).with_context(|| format!("invalid config {}", path.display()))
}

fn load_legacy(config_root: &Path) -> Result<DeviceConfig> {
    let path = legacy_config_path(config_root)?;
    if !path.exists() {
        anyhow::bail!("no legacy device config at {}", path.display());
    }
    let content = std::fs::read(&path)
        .with_context(|| format!("failed to read legacy config {}", path.display()))?;
    let legacy: LegacyDeviceConfig = serde_json::from_slice(&content)
        .with_context(|| format!("failed to parse legacy config {}", path.display()))?;
    Ok(migrate_legacy(legacy).validated())
}

fn migrate_legacy(legacy: LegacyDeviceConfig) -> DeviceConfig {
    DeviceConfig {
        preferred_brightness: legacy
            .preferred_brightness
            .into_iter()
            .map(|(id, brightness)| (id, BrightnessPreference { brightness }))
            .collect(),
        policy: legacy
            .policy
            .into_iter()
            .map(|(id, policy)| (id, PolicySelection { policy }))
            .collect(),
    }
}

fn parse_store(json: &serde_json::Value) -> Result<DeviceConfig> {
    let config: DeviceConfig =
        qol_config::deserialize_with_contract_defaults(CONFIG_CONTRACT, json.clone()).map_err(
            |errors| {
                anyhow::anyhow!(errors
                    .iter()
                    .map(|error| error.to_string())
                    .collect::<Vec<_>>()
                    .join("; "))
            },
        )?;
    Ok(config.validated())
}

pub fn save(config: &DeviceConfig) -> Result<()> {
    if qol_runtime::plugin_config::save(config) {
        Ok(())
    } else {
        anyhow::bail!("{PLUGIN_ID}: failed to persist config over the runtime socket")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let config_root = dir.path().join("config").join("qol-tray");
        std::fs::create_dir_all(config_root.join("profile").join("default")).unwrap();
        (dir, config_root)
    }

    fn with_active_profile(config_root: &Path, name: &str) {
        let profile = profile_root(config_root);
        std::fs::create_dir_all(&profile).unwrap();
        std::fs::write(profile.join("active"), name).unwrap();
    }

    #[test]
    fn contract_defaults_match_the_runtime_type() {
        qol_config::validate_contract_defaults_match_type::<DeviceConfig>(CONFIG_CONTRACT).unwrap();
        let defaults: DeviceConfig =
            qol_config::typed_defaults_from_contract(CONFIG_CONTRACT).unwrap();
        assert_eq!(defaults.preferred_brightness.len(), 0);
        assert_eq!(defaults.policy.len(), 0);
        assert_eq!(
            defaults,
            DeviceConfig::default(),
            "the contract defaults and the runtime defaults must not drift apart"
        );
    }

    #[test]
    fn device_paths_stay_under_the_profile_device_scope() {
        let (_dir, config_root) = root();
        let device = device_dir(&config_root).unwrap();
        assert!(device.starts_with(config_root.join("profile").join("default").join("device")));
        assert_eq!(
            device.file_name().unwrap(),
            "monitor",
            "the device namespace is monitor"
        );
        assert_eq!(
            legacy_config_path(&config_root).unwrap(),
            device.join(DEVICE_CONFIG_FILE)
        );
        assert_eq!(
            session_dir(&config_root).unwrap(),
            device.join(SESSION_SUBDIR)
        );
    }

    #[test]
    fn hotkeys_path_is_os_scoped_in_the_profile() {
        let (_dir, config_root) = root();
        let bucket = qol_profile_sync::current_os_bucket();
        assert_eq!(
            hotkeys_path(&config_root).unwrap(),
            config_root
                .join("profile")
                .join("default")
                .join("os")
                .join(bucket)
                .join("hotkeys.json")
        );
    }

    #[test]
    fn active_profile_name_defaults_and_falls_back() {
        let (_dir, config_root) = root();
        assert_eq!(active_profile_name(&config_root), "default");
        with_active_profile(&config_root, "work");
        assert_eq!(active_profile_name(&config_root), "work");
    }

    #[test]
    fn unsafe_profile_marker_falls_back_to_default() {
        let (_dir, config_root) = root();
        with_active_profile(&config_root, "../escape");
        assert_eq!(
            active_profile_name(&config_root),
            "default",
            "an unsafe marker must never escape the profile root"
        );
        assert!(device_dir(&config_root).is_ok());
    }

    #[test]
    fn config_round_trips_through_the_contract_shape() {
        let json = serde_json::json!({
            "preferred_brightness": {
                "id-a": { "brightness": 80 },
                "id-b": { "brightness": 45 },
            },
            "policy": {
                "id-a": { "policy": "gamma" },
                "id-b": { "policy": "off" },
            },
        });
        let config = parse_store(&json).unwrap();
        assert_eq!(config.preferred_for("id-a"), Some(80));
        assert_eq!(config.preferred_for("id-b"), Some(45));
        assert_eq!(config.policy_for("id-a"), BrightnessPolicy::Gamma);
        assert_eq!(config.policy_for("id-b"), BrightnessPolicy::Off);
        let round = serde_json::to_value(&config).unwrap();
        assert_eq!(round, json);
    }

    #[test]
    fn load_from_defaults_when_every_source_is_absent() {
        let (_dir, config_root) = root();
        let (config, origin) = load_from(None, None, Some(&config_root));
        assert_eq!(config, DeviceConfig::default());
        assert_eq!(origin, ConfigOrigin::Defaults);
    }

    #[test]
    fn load_from_prefers_the_host_store() {
        let (_dir, config_root) = root();
        let store = serde_json::json!({ "policy": { "id-a": { "policy": "ddc" } } });
        let materialized = tempfile::tempdir().unwrap();
        let path = materialized.path().join("config.json");
        std::fs::write(&path, serde_json::to_vec(&store).unwrap()).unwrap();
        let (config, origin) = load_from(Some(store), Some(&path), Some(&config_root));
        assert_eq!(origin, ConfigOrigin::HostStore);
        assert_eq!(config.policy_for("id-a"), BrightnessPolicy::Ddc);
    }

    #[test]
    fn load_from_falls_back_to_the_materialized_file() {
        let (_dir, config_root) = root();
        let materialized = tempfile::tempdir().unwrap();
        let path = materialized.path().join("config.json");
        std::fs::write(
            &path,
            serde_json::json!({ "preferred_brightness": { "id-a": { "brightness": 30 } } })
                .to_string(),
        )
        .unwrap();
        let (config, origin) = load_from(None, Some(&path), Some(&config_root));
        assert_eq!(origin, ConfigOrigin::MaterializedFile);
        assert_eq!(config.preferred_for("id-a"), Some(30));
    }

    #[test]
    fn load_from_adopts_the_legacy_flat_device_config() {
        let (_dir, config_root) = root();
        let legacy = legacy_config_path(&config_root).unwrap();
        std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
        std::fs::write(
            &legacy,
            serde_json::json!({
                "preferred_brightness": { "id-a": 80 },
                "policy": { "id-a": "gamma" },
            })
            .to_string(),
        )
        .unwrap();
        let (config, origin) = load_from(None, None, Some(&config_root));
        assert_eq!(origin, ConfigOrigin::LegacyAdopted);
        assert_eq!(config.preferred_for("id-a"), Some(80));
        assert_eq!(config.policy_for("id-a"), BrightnessPolicy::Gamma);
    }

    #[test]
    fn validation_drops_out_of_range_and_unknown_values() {
        let json = serde_json::json!({
            "preferred_brightness": {
                "id-a": { "brightness": 101 },
                "id-b": { "brightness": 77 },
            },
            "policy": {
                "id-a": { "policy": "night" },
                "id-b": { "policy": "ddc" },
            },
        });
        let config = parse_store(&json).unwrap();
        assert_eq!(config.preferred_for("id-a"), None);
        assert_eq!(config.preferred_for("id-b"), Some(77));
        assert_eq!(config.policy_for("id-a"), BrightnessPolicy::Auto);
        assert_eq!(config.policy_for("id-b"), BrightnessPolicy::Ddc);
    }

    #[test]
    fn policy_for_resolves_a_per_display_selection() {
        let config = DeviceConfig {
            policy: BTreeMap::from([(
                "id-a".to_string(),
                PolicySelection {
                    policy: "gamma".into(),
                },
            )]),
            ..DeviceConfig::default()
        };
        assert_eq!(config.policy_for("id-a"), BrightnessPolicy::Gamma);
        assert_eq!(config.policy_for("id-b"), BrightnessPolicy::Auto);
    }
}
