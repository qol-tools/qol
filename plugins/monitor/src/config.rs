use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use qol_profile_sync::{device_local_dir, SyncPaths};

use crate::monitor::BrightnessPolicy;

pub const DEVICE_NAMESPACE: &str = "monitor";
pub const DEVICE_CONFIG_FILE: &str = "config.json";
pub const SESSION_SUBDIR: &str = "session";
const PREFERRED_MAX: u8 = 100;

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DeviceConfig {
    #[serde(default)]
    pub preferred_brightness: BTreeMap<String, u8>,
    #[serde(default)]
    pub policy: BTreeMap<String, String>,
}

impl DeviceConfig {
    pub fn validated(mut self) -> Self {
        self.preferred_brightness
            .retain(|_, value| *value <= PREFERRED_MAX);
        self.policy
            .retain(|_, label| BrightnessPolicy::parse(label).is_some());
        self
    }

    pub fn policy_for(&self, display_id: &str) -> BrightnessPolicy {
        self.policy
            .get(display_id)
            .and_then(|label| BrightnessPolicy::parse(label))
            .unwrap_or_default()
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

pub fn config_path(config_root: &Path) -> Result<PathBuf> {
    device_dir(config_root).map(|dir| dir.join(DEVICE_CONFIG_FILE))
}

pub fn session_dir(config_root: &Path) -> Result<PathBuf> {
    device_dir(config_root).map(|dir| dir.join(SESSION_SUBDIR))
}

pub fn hotkeys_path(config_root: &Path) -> Result<PathBuf> {
    Ok(SyncPaths::new(profile_root(config_root)).hotkeys_path())
}

pub fn load(config_root: &Path) -> Result<DeviceConfig> {
    let path = config_path(config_root)?;
    if !path.exists() {
        return Ok(DeviceConfig::default());
    }
    let content = std::fs::read(&path)
        .with_context(|| format!("failed to read device config {}", path.display()))?;
    let config: DeviceConfig = serde_json::from_slice(&content)
        .with_context(|| format!("failed to parse device config {}", path.display()))?;
    Ok(config.validated())
}

pub fn save(config_root: &Path, config: &DeviceConfig) -> Result<()> {
    let config = config.clone().validated();
    let path = config_path(config_root)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create device config dir {}", parent.display()))?;
    }
    let content =
        serde_json::to_vec_pretty(&config).context("failed to serialize device config")?;
    qol_fs::atomic_write_durable_mode(&path, &content, 0o644)
        .with_context(|| format!("failed to commit device config {}", path.display()))
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
            config_path(&config_root).unwrap(),
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
    fn config_round_trips_through_the_device_scope() {
        let (_dir, config_root) = root();
        let config = DeviceConfig {
            preferred_brightness: BTreeMap::from([
                ("id-a".to_string(), 80),
                ("id-b".to_string(), 45),
            ]),
            policy: BTreeMap::from([
                ("id-a".to_string(), "gamma".to_string()),
                ("id-b".to_string(), "off".to_string()),
            ]),
        };
        save(&config_root, &config).unwrap();
        let loaded = load(&config_root).unwrap();
        assert_eq!(loaded, config);
        let written = std::fs::read_to_string(config_path(&config_root).unwrap()).unwrap();
        assert!(written.contains("\"id-a\""));
    }

    #[test]
    fn load_is_a_no_op_when_no_config_exists() {
        let (_dir, config_root) = root();
        assert_eq!(load(&config_root).unwrap(), DeviceConfig::default());
    }

    #[test]
    fn validation_drops_out_of_range_and_unknown_values() {
        let config = DeviceConfig {
            preferred_brightness: BTreeMap::from([
                ("id-a".to_string(), 101),
                ("id-b".to_string(), 77),
            ]),
            policy: BTreeMap::from([
                ("id-a".to_string(), "night".to_string()),
                ("id-b".to_string(), "ddc".to_string()),
            ]),
        }
        .validated();
        assert_eq!(
            config.preferred_brightness,
            BTreeMap::from([("id-b".to_string(), 77)])
        );
        assert_eq!(
            config.policy,
            BTreeMap::from([("id-b".to_string(), "ddc".to_string())])
        );
    }

    #[test]
    fn policy_for_resolves_a_per_display_selection() {
        let config = DeviceConfig {
            policy: BTreeMap::from([("id-a".to_string(), "gamma".to_string())]),
            ..DeviceConfig::default()
        };
        assert_eq!(config.policy_for("id-a"), BrightnessPolicy::Gamma);
        assert_eq!(config.policy_for("id-b"), BrightnessPolicy::Auto);
    }
}
