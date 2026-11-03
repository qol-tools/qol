use anyhow::Result;
use std::path::Path;

use crate::device_fields::{self, PluginFields};
use crate::{FileMigration, MigrationReport};

pub(crate) struct V3_137ToV3_138KeyremapBackendDevice;

const NAME: &str = "v3.137-to-v3.138-keyremap-backend-device";

const DEVICE_FIELDS: PluginFields = &[("qol-keyremap", &["key_backend"])];

impl FileMigration for V3_137ToV3_138KeyremapBackendDevice {
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
