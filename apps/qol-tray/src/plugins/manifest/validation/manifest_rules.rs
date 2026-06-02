use crate::plugins::manifest::{PluginManifest, CURRENT_MANIFEST_VERSION};
use anyhow::{bail, Context, Result};

impl PluginManifest {
    pub fn validate(&self) -> Result<()> {
        self.validate_version()?;
        let action_ids = super::menu_rules::collect_menu_action_ids(&self.menu.items)?;
        super::runtime_rules::validate_optional_runtime_config(
            self.runtime.as_ref(),
            &action_ids.executable,
        )?;
        super::command_rules::validate_optional_daemon_config(self.daemon.as_ref())?;
        super::dependency_rules::validate_optional_dependencies(self.dependencies.as_ref())?;
        Ok(())
    }

    pub fn validate_version(&self) -> Result<()> {
        validate_manifest_version(self.manifest_version)
    }

    pub fn load_and_validate(path: impl AsRef<std::path::Path>) -> Result<Self> {
        let path = path.as_ref();
        let raw =
            std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
        let manifest: Self =
            toml::from_str(&raw).with_context(|| format!("parse {}", path.display()))?;
        manifest
            .validate()
            .with_context(|| format!("validate {}", path.display()))?;
        Ok(manifest)
    }
}

const MIN_SUPPORTED_MANIFEST_VERSION: u32 = 1;

fn validate_manifest_version(version: u32) -> Result<()> {
    if (MIN_SUPPORTED_MANIFEST_VERSION..=CURRENT_MANIFEST_VERSION).contains(&version) {
        return Ok(());
    }

    bail!(
        "Unsupported manifest_version {} (expected {}..={})",
        version,
        MIN_SUPPORTED_MANIFEST_VERSION,
        CURRENT_MANIFEST_VERSION
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_current_version() {
        validate_manifest_version(CURRENT_MANIFEST_VERSION).unwrap();
    }

    #[test]
    fn accepts_minimum_supported_version() {
        validate_manifest_version(MIN_SUPPORTED_MANIFEST_VERSION).unwrap();
    }

    #[test]
    fn rejects_below_minimum() {
        assert!(validate_manifest_version(0).is_err());
    }

    #[test]
    fn rejects_above_current() {
        assert!(validate_manifest_version(CURRENT_MANIFEST_VERSION + 1).is_err());
    }
}
