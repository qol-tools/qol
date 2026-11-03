use anyhow::Result;
use std::path::Path;

use crate::device_fields::{self, PluginFields};
use crate::{FileMigration, MigrationReport};

pub(crate) struct V3_115ToV3_116DevicePluginFields;

const NAME: &str = "v3.115-to-v3.116-device-plugin-fields";

const DEVICE_FIELDS: PluginFields = &[
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
        device_fields::applies(config_dir, DEVICE_FIELDS)
    }

    fn migrate(&self, config_dir: &Path, archive_dir: &Path) -> Result<MigrationReport> {
        device_fields::migrate(config_dir, archive_dir, NAME, DEVICE_FIELDS)
    }
}

#[cfg(test)]
mod tests;
