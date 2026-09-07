//! Profile schema repairs applied on the sync path.
//!
//! Both sync entry points must run the same migrations before pushing so a
//! profile that still carries legacy plugin ids is normalized identically no
//! matter which side touches it first.

use anyhow::{anyhow, Result};
use qol_migrations::FileMigration;
use std::path::Path;

/// Runs the plugin-uid schema repair the tray applies on every sync. Returns
/// whether a repair was actually applied.
pub fn repair_profile_schema(profile_root: &Path) -> Result<bool> {
    let config_dir = profile_root.parent().ok_or_else(|| {
        anyhow!(
            "profile repo {} has no config parent",
            profile_root.display()
        )
    })?;
    let migration = qol_migrations::V3_19ToV3_20PluginUid::default_for_production();
    if !migration.applies(config_dir)? {
        return Ok(false);
    }
    migration.migrate(config_dir, config_dir)?;
    Ok(true)
}
