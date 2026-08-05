use anyhow::{Context, Result};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const REGISTRY_FILE: &str = "devices.json";
const QUARANTINE_EXTENSION: &str = "unreadable";

pub struct DeviceRecord {
    pub key: [u8; 32],
    pub name: String,
    pub paired_at_ms: u64,
}

pub struct DeviceRegistry {
    path: PathBuf,
    devices: HashMap<[u8; 16], DeviceRecord>,
}

#[derive(Serialize, Deserialize, Default)]
struct StoredRegistry {
    devices: Vec<StoredDevice>,
}

#[derive(Serialize, Deserialize)]
struct StoredDevice {
    device_id: String,
    key: String,
    name: String,
    paired_at_ms: u64,
}

impl DeviceRegistry {
    pub fn load() -> Result<Self> {
        let path = registry_path().context("PointZ data directory is unavailable")?;
        Self::load_at(path)
    }

    pub(crate) fn load_at(path: PathBuf) -> Result<Self> {
        let devices = match std::fs::read(&path) {
            Ok(bytes) => match decode_stored(&bytes) {
                Ok(devices) => devices,
                Err(error) => {
                    quarantine(&path, &error)?;
                    HashMap::new()
                }
            },
            Err(error) if error.kind() == ErrorKind::NotFound => HashMap::new(),
            Err(error) => return Err(error.into()),
        };
        Ok(Self { path, devices })
    }

    pub fn key_for(&self, device_id: &[u8; 16]) -> Option<[u8; 32]> {
        self.devices.get(device_id).map(|record| record.key)
    }

    pub fn upsert(&mut self, device_id: [u8; 16], key: [u8; 32], name: String) -> Result<()> {
        self.devices.insert(
            device_id,
            DeviceRecord {
                key,
                name,
                paired_at_ms: unix_time_ms(),
            },
        );
        self.persist()
    }

    fn persist(&self) -> Result<()> {
        let stored = StoredRegistry {
            devices: self
                .devices
                .iter()
                .map(|(device_id, record)| StoredDevice {
                    device_id: URL_SAFE_NO_PAD.encode(device_id),
                    key: URL_SAFE_NO_PAD.encode(record.key),
                    name: record.name.clone(),
                    paired_at_ms: record.paired_at_ms,
                })
                .collect(),
        };
        let json = serde_json::to_vec_pretty(&stored)
            .context("failed to serialize the PointZ device registry")?;
        qol_fs::atomic_write_private(&self.path, &json).with_context(|| {
            format!(
                "failed to persist the PointZ device registry at {}",
                self.path.display()
            )
        })
    }
}

fn decode_stored(bytes: &[u8]) -> Result<HashMap<[u8; 16], DeviceRecord>> {
    let stored: StoredRegistry =
        serde_json::from_slice(bytes).context("PointZ device registry is not valid JSON")?;
    let mut devices = HashMap::with_capacity(stored.devices.len());
    for device in stored.devices {
        let device_id = decode_array::<16>(&device.device_id, "device id")?;
        let key = decode_array::<32>(&device.key, "device key")?;
        devices.insert(
            device_id,
            DeviceRecord {
                key,
                name: device.name,
                paired_at_ms: device.paired_at_ms,
            },
        );
    }
    Ok(devices)
}

fn decode_array<const N: usize>(encoded: &str, label: &str) -> Result<[u8; N]> {
    let decoded = URL_SAFE_NO_PAD
        .decode(encoded)
        .with_context(|| format!("device registry {label} is not valid base64url"))?;
    decoded
        .try_into()
        .map_err(|_| anyhow::anyhow!("device registry {label} has the wrong length"))
}

fn quarantine(path: &Path, reason: &anyhow::Error) -> Result<()> {
    let quarantined = path.with_extension(QUARANTINE_EXTENSION);
    std::fs::rename(path, &quarantined).with_context(|| {
        format!(
            "failed to set aside the unreadable PointZ device registry at {}",
            path.display()
        )
    })?;
    log::warn!(
        "Starting with an empty PointZ device registry ({reason}); the old file is at {} and every device must pair again",
        quarantined.display()
    );
    Ok(())
}

fn registry_path() -> Option<PathBuf> {
    let plugin_id = qol_config::plugin_id_from_env(env!("QOL_PLUGIN_ID"));
    Some(
        qol_config::data_dir()?
            .join("plugins")
            .join(plugin_id)
            .join(REGISTRY_FILE),
    )
}

fn unix_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry(root: &Path) -> DeviceRegistry {
        DeviceRegistry::load_at(root.join("devices.json")).unwrap()
    }

    #[test]
    fn an_upserted_device_key_survives_a_reload() {
        let root = tempfile::tempdir().unwrap();
        let mut first = registry(root.path());

        first.upsert([1; 16], [9; 32], "Pixel".to_string()).unwrap();
        let reloaded = registry(root.path());

        assert_eq!(reloaded.key_for(&[1; 16]), Some([9; 32]));
        assert_eq!(reloaded.key_for(&[2; 16]), None);
    }

    #[test]
    fn an_overwritten_device_keeps_only_its_newest_key() {
        let root = tempfile::tempdir().unwrap();
        let mut reg = registry(root.path());

        reg.upsert([1; 16], [9; 32], "Pixel".to_string()).unwrap();
        reg.upsert([1; 16], [4; 32], "Pixel".to_string()).unwrap();

        assert_eq!(registry(root.path()).key_for(&[1; 16]), Some([4; 32]));
    }

    #[test]
    fn a_corrupt_registry_is_set_aside_and_starts_empty() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("devices.json");
        std::fs::write(&path, "NOT JSON").unwrap();

        let reg = DeviceRegistry::load_at(path.clone()).unwrap();

        assert_eq!(reg.key_for(&[1; 16]), None);
        assert_eq!(
            std::fs::read_to_string(path.with_extension("unreadable")).unwrap(),
            "NOT JSON"
        );
    }
}
