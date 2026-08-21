use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const RESIDENCY_FILE: &str = "residency.json";
pub const RESIDENCY_NAMESPACE: &str = "residency";
pub const PROFILE_FILE: &str = "active";
pub const DEFAULT_PROFILE: &str = "default";
pub const CORE_SUBDIR: &str = "core";
pub const DEVICE_ID_REMAP: &str = "QOL_RESIDENCY_DEVICE_ID";
pub const CONFIG_DIR_REMAP: &str = "QOL_RESIDENCY_CONFIG_DIR";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HostResidency {
    #[default]
    Portable,
    Resident,
}

impl HostResidency {
    pub fn is_resident(self) -> bool {
        matches!(self, Self::Resident)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Portable => "portable",
            Self::Resident => "resident",
        }
    }

    pub fn current() -> Self {
        config_root()
            .and_then(|root| {
                current_device_id()
                    .ok()
                    .and_then(|id| Self::read_device(&root, &id).ok())
            })
            .unwrap_or_default()
    }

    pub fn set(value: Self) -> Result<()> {
        set_current_device_residency(value).map(drop)
    }

    pub fn read_from(config_root: &Path) -> Self {
        current_device_id()
            .ok()
            .and_then(|id| Self::read_device(config_root, &id).ok())
            .unwrap_or_default()
    }

    pub fn write_to(config_root: &Path, value: Self) -> Result<()> {
        let id = current_device_id()?;
        Self::write_device(config_root, &id, value)
    }

    pub fn read_device(config_root: &Path, device_id: &str) -> Result<Self> {
        let map = read_map(config_root)?;
        Ok(map.get(device_id).copied().unwrap_or_default())
    }

    pub fn write_device(config_root: &Path, device_id: &str, value: Self) -> Result<()> {
        let mut map = read_map(config_root)?;
        match value {
            Self::Resident => {
                map.insert(device_id.to_string(), value);
            }
            Self::Portable => {
                map.remove(device_id);
            }
        }
        write_map(config_root, &map)
    }
}

pub fn set_current_device_residency(value: HostResidency) -> Result<String> {
    let root = config_root().context("no config directory to store residency in")?;
    let id = current_device_id()?;
    HostResidency::write_device(&root, &id, value)?;
    Ok(id)
}

pub fn residency_path(config_root: &Path) -> PathBuf {
    profile_root(config_root)
        .join(active_profile(config_root))
        .join(CORE_SUBDIR)
        .join(RESIDENCY_NAMESPACE)
        .join(RESIDENCY_FILE)
}

pub fn read_map(config_root: &Path) -> Result<BTreeMap<String, HostResidency>> {
    let path = residency_path(config_root);
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return Ok(BTreeMap::new());
    };
    serde_json::from_str(&raw).with_context(|| format!("failed to parse {}", path.display()))
}

pub fn write_map(config_root: &Path, map: &BTreeMap<String, HostResidency>) -> Result<()> {
    let path = residency_path(config_root);
    let parent = path
        .parent()
        .context("residency path has no parent directory")?;
    std::fs::create_dir_all(parent)
        .with_context(|| format!("failed to create {}", parent.display()))?;
    let body = serde_json::to_vec_pretty(map)?;
    qol_fs::atomic_write_durable(&path, &body)
        .with_context(|| format!("failed to write {}", path.display()))
}

pub fn profile_root(config_root: &Path) -> PathBuf {
    config_root.join("profile")
}

pub fn active_profile(config_root: &Path) -> String {
    let marker = profile_root(config_root).join(PROFILE_FILE);
    std::fs::read_to_string(marker)
        .ok()
        .map(|raw| raw.trim().to_string())
        .filter(|name| is_safe_profile_name(name))
        .unwrap_or_else(|| DEFAULT_PROFILE.to_string())
}

fn is_safe_profile_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name != "."
        && name != ".."
        && !name.contains('/')
        && !name.contains('\\')
}

fn config_root() -> Option<PathBuf> {
    std::env::var_os(CONFIG_DIR_REMAP)
        .map(PathBuf::from)
        .or_else(qol_config::config_dir)
}

fn current_device_id() -> Result<String> {
    if let Some(raw) = std::env::var_os(DEVICE_ID_REMAP) {
        let id = raw.to_string_lossy().into_owned();
        if id.is_empty() {
            bail!("QOL_RESIDENCY_DEVICE_ID must not be empty");
        }
        return Ok(id);
    }
    platform_device_id()
}

#[cfg(target_os = "linux")]
fn platform_device_id() -> Result<String> {
    let owner = crate::policy::stable_host_owner("residency")?;
    Ok(owner.as_str().to_string())
}

#[cfg(target_os = "macos")]
fn platform_device_id() -> Result<String> {
    macos_device_id()
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn platform_device_id() -> Result<String> {
    bail!("no device residency identity derived on this platform")
}

#[cfg(target_os = "macos")]
fn macos_device_id() -> Result<String> {
    let raw = std::process::Command::new("ioreg")
        .args(["-rd1", "-c", "IOPlatformExpertDevice"])
        .output()
        .context("failed to run ioreg for the residency device identity")?;
    if !raw.status.success() {
        bail!("ioreg failed to enumerate the platform device");
    }
    let output = String::from_utf8_lossy(&raw.stdout);
    let uuid = parse_platform_uuid(&output).context("failed to parse IOPlatformUUID")?;
    Ok(derive_device_id(&uuid))
}

#[cfg(target_os = "macos")]
fn derive_device_id(source: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update("residency".as_bytes());
    hasher.update(b":");
    hasher.update(source.as_bytes());
    let digest = format!("{:x}", hasher.finalize());
    format!("qol-resident-{}", &digest[..16])
}

#[cfg(target_os = "macos")]
fn parse_platform_uuid(output: &str) -> Result<String> {
    for line in output.lines() {
        let trimmed = line.trim();
        if !trimmed.contains("IOPlatformUUID") {
            continue;
        }
        let Some(rhs) = trimmed.split('=').nth(1) else {
            continue;
        };
        let uuid = rhs.trim().trim_matches('"').trim();
        if !uuid.is_empty() {
            return Ok(uuid.to_string());
        }
    }
    bail!("no IOPlatformUUID found in the ioreg output")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config_root_from(dir: &Path) -> PathBuf {
        dir.join("config").join("qol-tray")
    }

    fn with_active_profile(config_root: &Path, name: &str) {
        let profile = profile_root(config_root);
        std::fs::create_dir_all(&profile).unwrap();
        std::fs::write(profile.join(PROFILE_FILE), name).unwrap();
    }

    fn with_device(config_root: &Path) {
        with_active_profile(config_root, "default");
    }

    #[test]
    fn a_fresh_host_is_portable() {
        let dir = tempfile::tempdir().unwrap();
        let root = config_root_from(dir.path());
        with_device(&root);

        let map = read_map(&root).unwrap();
        assert!(
            map.is_empty(),
            "a fresh profile carries no residency entries"
        );
    }

    #[test]
    fn residency_survives_a_write_and_read() {
        let dir = tempfile::tempdir().unwrap();
        let root = config_root_from(dir.path());
        with_device(&root);

        HostResidency::write_device(&root, "test-device", HostResidency::Resident).unwrap();

        let map = read_map(&root).unwrap();
        assert_eq!(
            map.get("test-device").copied(),
            Some(HostResidency::Resident),
            "the device entry is a map of device id to residency"
        );
        assert!(
            HostResidency::read_device(&root, "test-device")
                .unwrap()
                .is_resident(),
            "the persistent entry survives a read"
        );
    }

    #[test]
    fn residency_lives_inside_the_profile_tree_so_sync_carries_it() {
        let dir = tempfile::tempdir().unwrap();
        let root = config_root_from(dir.path());
        with_device(&root);

        let path = residency_path(&root);

        assert!(
            path.starts_with(root.join("profile").join("default").join("core")),
            "the residency file must live in the profile's synced core subtree, got {}",
            path.display()
        );
        assert_eq!(
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned()),
            Some(RESIDENCY_FILE.to_string())
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_platform_uuid_parser_extracts_a_quoted_uuid() {
        let output = "    \"IOPlatformUUID\" = \"AAAABBBB-CCCC-DDDD-EEEE-FFFF0099AABB\"\n";
        assert_eq!(
            parse_platform_uuid(output).unwrap(),
            "AAAABBBB-CCCC-DDDD-EEEE-FFFF0099AABB"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_platform_uuid_parser_rejects_missing_or_empty_values() {
        let missing = "    \"IOPlatformName\" = \"Mac\"\n";
        assert!(parse_platform_uuid(missing).is_err());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_device_id_is_machine_derived_and_stable_in_shape() {
        let id = derive_device_id("AAAABBBB-CCCC-DDDD-EEEE-FFFF0099AABB");
        assert!(id.starts_with("qol-resident-"), "{id}");
        assert_eq!(id.len(), "qol-resident-".len() + 16, "{id}");
    }

    #[test]
    fn turning_residency_back_off_drops_the_device_entry() {
        let dir = tempfile::tempdir().unwrap();
        let root = config_root_from(dir.path());
        with_device(&root);

        HostResidency::write_device(&root, "test-device", HostResidency::Resident).unwrap();
        HostResidency::write_device(&root, "test-device", HostResidency::Portable).unwrap();

        let map = read_map(&root).unwrap();
        assert!(!map.contains_key("test-device"));
        assert_eq!(
            HostResidency::read_device(&root, "test-device").unwrap(),
            HostResidency::Portable
        );
    }

    #[test]
    fn an_unsafe_profile_marker_falls_back_to_default() {
        let dir = tempfile::tempdir().unwrap();
        let root = config_root_from(dir.path());
        with_active_profile(&root, "../escape");

        assert_eq!(active_profile(&root), "default");
        assert!(residency_path(&root).starts_with(root.join("profile").join("default")));
    }

    #[test]
    fn a_missing_entry_means_portable_for_a_foreign_device() {
        let dir = tempfile::tempdir().unwrap();
        let root = config_root_from(dir.path());
        with_device(&root);
        HostResidency::write_device(&root, "my-box", HostResidency::Resident).unwrap();

        assert_eq!(
            HostResidency::read_device(&root, "my-box").unwrap(),
            HostResidency::Resident
        );
        assert_eq!(
            HostResidency::read_device(&root, "other-box").unwrap(),
            HostResidency::Portable,
            "a device without an entry is portable"
        );
    }
}
