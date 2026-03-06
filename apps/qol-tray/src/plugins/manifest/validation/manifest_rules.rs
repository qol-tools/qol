use crate::plugins::manifest::{PluginManifest, CURRENT_MANIFEST_VERSION};
use anyhow::{bail, Result};

impl PluginManifest {
    pub fn validate(&self) -> Result<()> {
        validate_manifest_version(self.manifest_version)?;
        let action_ids = super::menu_rules::collect_menu_action_ids(&self.menu.items)?;
        super::runtime_rules::validate_optional_runtime_config(
            self.runtime.as_ref(),
            &action_ids.executable,
        )?;
        super::command_rules::validate_optional_daemon_config(self.daemon.as_ref())?;
        super::dependency_rules::validate_optional_dependencies(self.dependencies.as_ref())?;
        Ok(())
    }
}

fn validate_manifest_version(version: u32) -> Result<()> {
    if version == CURRENT_MANIFEST_VERSION {
        return Ok(());
    }

    bail!(
        "Unsupported manifest_version {} (expected {})",
        version,
        CURRENT_MANIFEST_VERSION
    )
}
