use std::collections::BTreeMap;

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde::Deserialize;
use zeroize::Zeroizing;

use super::{seed::Seed, MAX_DEVICES};
use crate::pointz::{PointzDeviceId, PointzImport, PointzImportSource};
use crate::service::AuthorityError;

pub(crate) struct LegacyDevice {
    pub(crate) id: PointzDeviceId,
    pub(crate) key: Zeroizing<[u8; 32]>,
    pub(crate) name: String,
    pub(crate) paired_at_ms: u64,
}

pub struct LegacyPointz {
    pub(crate) seed: Seed,
    pub(crate) devices: Vec<LegacyDevice>,
    pub(crate) import: PointzImport,
}

#[derive(Deserialize)]
struct StoredRegistry {
    devices: Vec<StoredDevice>,
}

#[derive(Deserialize)]
struct StoredDevice {
    device_id: String,
    key: String,
    name: String,
    paired_at_ms: u64,
}

impl LegacyPointz {
    pub fn decode(seed: Option<&[u8]>, devices: Option<&[u8]>) -> Result<Self, AuthorityError> {
        let stored_seed = seed.and_then(decode_seed);
        let seed_replaced = stored_seed.is_none();
        let seed = match stored_seed {
            Some(seed) => seed,
            None => Seed::generate().ok_or(AuthorityError::Identity)?,
        };
        let decoded = devices.map(decode_devices);
        let devices_unreadable = matches!(decoded, Some(None));
        let mut devices = decoded.flatten().unwrap_or_default();
        devices.sort_by_key(|device| std::cmp::Reverse(device.paired_at_ms));
        let dropped = devices.len().saturating_sub(MAX_DEVICES);
        devices.truncate(MAX_DEVICES);
        Ok(Self {
            seed,
            import: PointzImport {
                source: PointzImportSource::Legacy,
                imported: devices.len() as u32,
                dropped: dropped as u32,
                seed_replaced,
                devices_unreadable,
            },
            devices,
        })
    }

    pub fn import(&self) -> PointzImport {
        self.import
    }
}

fn decode_seed(bytes: &[u8]) -> Option<Seed> {
    let text = std::str::from_utf8(bytes).ok()?;
    decode_array::<32>(text.trim()).map(Seed::from_bytes)
}

fn decode_devices(bytes: &[u8]) -> Option<Vec<LegacyDevice>> {
    let stored: StoredRegistry = serde_json::from_slice(bytes).ok()?;
    let mut newest = BTreeMap::new();
    for device in stored.devices {
        let id = PointzDeviceId::from_bytes(decode_array::<16>(&device.device_id)?);
        let key = Zeroizing::new(decode_array::<32>(&device.key)?);
        let candidate = LegacyDevice {
            id,
            key,
            name: device.name,
            paired_at_ms: device.paired_at_ms,
        };
        let replace = newest
            .get(&id)
            .is_none_or(|current: &LegacyDevice| current.paired_at_ms <= candidate.paired_at_ms);
        if replace {
            newest.insert(id, candidate);
        }
    }
    Some(newest.into_values().collect())
}

fn decode_array<const N: usize>(encoded: &str) -> Option<[u8; N]> {
    URL_SAFE_NO_PAD.decode(encoded).ok()?.try_into().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encoded(bytes: &[u8]) -> String {
        URL_SAFE_NO_PAD.encode(bytes)
    }

    fn registry(devices: &[([u8; 16], [u8; 32], &str, u64)]) -> Vec<u8> {
        let devices: Vec<_> = devices
            .iter()
            .map(|(id, key, name, paired_at_ms)| {
                serde_json::json!({
                    "device_id": encoded(id),
                    "key": encoded(key),
                    "name": name,
                    "paired_at_ms": paired_at_ms,
                })
            })
            .collect();
        serde_json::to_vec(&serde_json::json!({ "devices": devices })).unwrap()
    }

    #[test]
    fn legacy_seed_and_devices_import_unchanged() {
        let seed = format!("{}\n", encoded(&[9; 32]));
        let devices = registry(&[([1; 16], [2; 32], "Pixel", 10)]);

        let legacy = LegacyPointz::decode(Some(seed.as_bytes()), Some(&devices)).unwrap();

        assert_eq!(
            legacy.seed.server_id(),
            Seed::from_bytes([9; 32]).server_id()
        );
        assert_eq!(legacy.devices.len(), 1);
        assert_eq!(*legacy.devices[0].key, [2; 32]);
        assert_eq!(legacy.devices[0].name, "Pixel");
        assert!(!legacy.import.phones_must_pair_again());
        assert_eq!(legacy.import.imported, 1);
    }

    #[test]
    fn missing_or_unreadable_files_are_reported_and_never_guessed() {
        let missing_seed = LegacyPointz::decode(None, Some(&registry(&[]))).unwrap();
        let bad_seed = LegacyPointz::decode(Some(b"not base64"), None).unwrap();
        let bad_devices =
            LegacyPointz::decode(Some(encoded(&[9; 32]).as_bytes()), Some(b"NOT JSON")).unwrap();

        assert!(missing_seed.import.seed_replaced);
        assert!(bad_seed.import.seed_replaced);
        assert!(bad_devices.import.devices_unreadable);
        assert!(bad_devices.devices.is_empty());
        assert!(!bad_devices.import.seed_replaced);
    }

    #[test]
    fn a_device_list_with_an_invalid_entry_imports_none_of_it() {
        let mut devices: serde_json::Value =
            serde_json::from_slice(&registry(&[([1; 16], [2; 32], "Pixel", 10)])).unwrap();
        devices["devices"].as_array_mut().unwrap().push(
            serde_json::json!({"device_id": "short", "key": "", "name": "", "paired_at_ms": 1}),
        );

        let legacy = LegacyPointz::decode(
            Some(encoded(&[9; 32]).as_bytes()),
            Some(&serde_json::to_vec(&devices).unwrap()),
        )
        .unwrap();

        assert!(legacy.import.devices_unreadable);
        assert!(legacy.devices.is_empty());
    }

    #[test]
    fn only_the_newest_devices_fit_and_the_rest_are_counted() {
        let entries: Vec<_> = (0..(MAX_DEVICES as u8 + 3))
            .map(|index| ([index; 16], [index; 32], "Phone", u64::from(index)))
            .collect();

        let legacy = LegacyPointz::decode(
            Some(encoded(&[9; 32]).as_bytes()),
            Some(&registry(&entries)),
        )
        .unwrap();

        assert_eq!(legacy.devices.len(), MAX_DEVICES);
        assert_eq!(legacy.import.dropped, 3);
        assert!(legacy.devices.iter().all(|device| device.paired_at_ms >= 3));
    }
}
