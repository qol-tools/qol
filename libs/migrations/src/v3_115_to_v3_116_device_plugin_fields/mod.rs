use anyhow::Result;
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::fs_util::{
    archive_original, current_os_subdir, is_safe_path_component, list_profile_dirs,
    write_json_atomic,
};
use crate::{FileMigration, MigrationReport};

pub(crate) struct V3_115ToV3_116DevicePluginFields;

const NAME: &str = "v3.115-to-v3.116-device-plugin-fields";

const DEVICE_FIELDS: &[(&str, &[&str])] = &[
    ("qol-bluetooth", &["adapter", "managed_devices"]),
    ("qol-ide-checkout", &["tempDir"]),
    ("qol-launcher", &["extra_file_scan_roots"]),
    ("qol-memory", &["verifier_endpoint"]),
    ("qol-shot", &["audio.mic_device", "audio.system_device"]),
    (
        "qol-voice",
        &["audio.input_device", "recognition.model_dir"],
    ),
];

impl FileMigration for V3_115ToV3_116DevicePluginFields {
    fn name(&self) -> &'static str {
        NAME
    }

    fn applies(&self, config_dir: &Path) -> Result<bool> {
        for profile_dir in list_profile_dirs(&config_dir.join("profile"))? {
            for (uid, fields) in device_fields_by_uid(&profile_dir) {
                for path in synced_slices(&profile_dir, &uid) {
                    let Some(slice) = read_object(&path) else {
                        continue;
                    };
                    if fields.iter().any(|field| get_path(&slice, field).is_some()) {
                        return Ok(true);
                    }
                }
            }
        }
        Ok(false)
    }

    fn migrate(&self, config_dir: &Path, archive_dir: &Path) -> Result<MigrationReport> {
        let mut archived = Vec::new();
        for profile_dir in list_profile_dirs(&config_dir.join("profile"))? {
            for (uid, fields) in device_fields_by_uid(&profile_dir) {
                localize_plugin(
                    config_dir,
                    archive_dir,
                    &profile_dir,
                    &uid,
                    fields,
                    &mut archived,
                )?;
            }
        }
        Ok(MigrationReport {
            name: NAME.to_string(),
            archived,
        })
    }
}

fn localize_plugin(
    config_dir: &Path,
    archive_dir: &Path,
    profile_dir: &Path,
    uid: &str,
    fields: &[&str],
    archived: &mut Vec<PathBuf>,
) -> Result<()> {
    let device_path = slice_path(profile_dir.join("device"), uid);
    let original_device = read_object(&device_path).unwrap_or_default();
    let mut device = original_device.clone();
    let mut stripped = Vec::new();
    for path in synced_slices(profile_dir, uid).into_iter().rev() {
        let Some(original) = read_object(&path) else {
            continue;
        };
        let mut slice = original.clone();
        for field in fields {
            if let Some(value) = remove_path(&mut slice, field) {
                if get_path(&device, field).is_none() {
                    set_path(&mut device, field, value);
                }
            }
        }
        if slice != original {
            stripped.push((path, slice));
        }
    }
    if device != original_device {
        if device_path.is_file() {
            archived.push(archive_original(config_dir, archive_dir, &device_path)?);
        }
        write_json_atomic(&device_path, &Value::Object(device))?;
    }
    for (path, slice) in stripped {
        archived.push(archive_original(config_dir, archive_dir, &path)?);
        write_json_atomic(&path, &Value::Object(slice))?;
    }
    Ok(())
}

fn device_fields_by_uid(profile_dir: &Path) -> Vec<(String, &'static [&'static str])> {
    let lock = profile_dir.join("core").join("plugins.lock.json");
    let Some(lock) = read_object(&lock) else {
        return Vec::new();
    };
    let fields: HashMap<&str, &'static [&'static str]> = DEVICE_FIELDS.iter().copied().collect();
    let mut out = Vec::new();
    for entry in lock
        .get("plugins")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let id = entry.get("id").and_then(Value::as_str);
        let uid = entry.get("uid").and_then(Value::as_str);
        let (Some(id), Some(uid)) = (id, uid) else {
            continue;
        };
        if !is_safe_path_component(uid) || out.iter().any(|(seen, _)| seen == uid) {
            continue;
        }
        if let Some(fields) = fields.get(id) {
            out.push((uid.to_string(), *fields));
        }
    }
    out
}

fn synced_slices(profile_dir: &Path, uid: &str) -> [PathBuf; 2] {
    [
        slice_path(profile_dir.join("core"), uid),
        slice_path(profile_dir.join("os").join(current_os_subdir()), uid),
    ]
}

fn slice_path(scope_dir: PathBuf, uid: &str) -> PathBuf {
    scope_dir.join("plugin-configs").join(format!("{uid}.json"))
}

fn read_object(path: &Path) -> Option<Map<String, Value>> {
    let raw = std::fs::read_to_string(path).ok()?;
    match serde_json::from_str::<Value>(&raw) {
        Ok(Value::Object(map)) => Some(map),
        Ok(_) => None,
        Err(error) => {
            log::warn!("[{NAME}] skipping unparseable {}: {error}", path.display());
            None
        }
    }
}

fn get_path<'a>(map: &'a Map<String, Value>, field: &str) -> Option<&'a Value> {
    let mut parts = field.split('.');
    let mut value = map.get(parts.next()?)?;
    for part in parts {
        value = value.as_object()?.get(part)?;
    }
    Some(value)
}

fn remove_path(map: &mut Map<String, Value>, field: &str) -> Option<Value> {
    match field.split_once('.') {
        None => map.remove(field),
        Some((head, rest)) => {
            let child = map.get_mut(head)?.as_object_mut()?;
            let removed = remove_path(child, rest);
            if removed.is_some() && child.is_empty() {
                map.remove(head);
            }
            removed
        }
    }
}

fn set_path(map: &mut Map<String, Value>, field: &str, value: Value) {
    match field.split_once('.') {
        None => {
            map.insert(field.to_string(), value);
        }
        Some((head, rest)) => {
            let child = map
                .entry(head.to_string())
                .or_insert_with(|| Value::Object(Map::new()));
            if !child.is_object() {
                *child = Value::Object(Map::new());
            }
            if let Some(child) = child.as_object_mut() {
                set_path(child, rest, value);
            }
        }
    }
}

#[cfg(test)]
mod tests;
