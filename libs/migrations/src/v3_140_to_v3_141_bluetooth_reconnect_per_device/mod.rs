use anyhow::Result;
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};

use crate::device_fields::{self, PluginFields};
use crate::fs_util::{archive_original, list_profile_dirs, write_json_atomic};
use crate::{FileMigration, MigrationReport};

pub(crate) struct V3_140ToV3_141BluetoothReconnectPerDevice;

const NAME: &str = "v3.140-to-v3.141-bluetooth-reconnect-per-device";

const PLUGIN: PluginFields = &[("qol-bluetooth", &[])];
const SWITCH: &str = "auto_reconnect";
const CHOSEN: &str = "managed_devices";

impl FileMigration for V3_140ToV3_141BluetoothReconnectPerDevice {
    fn name(&self) -> &'static str {
        NAME
    }

    fn applies(&self, config_dir: &Path) -> Result<bool> {
        for profile_dir in list_profile_dirs(&config_dir.join("profile"))? {
            for (uid, _) in device_fields::device_fields_by_uid(&profile_dir, PLUGIN) {
                if stored_slices(&profile_dir, &uid)
                    .iter()
                    .any(|(_, slice)| slice.contains_key(SWITCH))
                {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    fn migrate(&self, config_dir: &Path, archive_dir: &Path) -> Result<MigrationReport> {
        let mut archived = Vec::new();
        for profile_dir in list_profile_dirs(&config_dir.join("profile"))? {
            for (uid, _) in device_fields::device_fields_by_uid(&profile_dir, PLUGIN) {
                let slices = stored_slices(&profile_dir, &uid);
                let switched_off = slices
                    .iter()
                    .find_map(|(_, slice)| slice.get(SWITCH))
                    .and_then(Value::as_bool)
                    == Some(false);
                for (path, original) in slices {
                    let mut slice = original.clone();
                    slice.remove(SWITCH);
                    if switched_off {
                        slice.remove(CHOSEN);
                    }
                    if slice == original {
                        continue;
                    }
                    archived.push(archive_original(config_dir, archive_dir, &path)?);
                    write_json_atomic(&path, &Value::Object(slice))?;
                }
            }
        }
        Ok(MigrationReport {
            name: NAME.to_string(),
            archived,
        })
    }
}

fn stored_slices(profile_dir: &Path, uid: &str) -> Vec<(PathBuf, Map<String, Value>)> {
    let [core, os] = device_fields::synced_slices(profile_dir, uid);
    [
        device_fields::slice_path(profile_dir.join("device"), uid),
        os,
        core,
    ]
    .into_iter()
    .filter_map(|path| {
        let slice = device_fields::read_object(&path)?;
        Some((path, slice))
    })
    .collect()
}

#[cfg(test)]
mod tests;
