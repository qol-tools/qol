use crate::manifest::{PluginManifest, CURRENT_MANIFEST_VERSION};
use anyhow::{bail, Context, Result};

impl PluginManifest {
    pub fn validate(&self) -> Result<()> {
        self.validate_version()?;
        self.plugin.validate_identity()?;
        let menu_action_ids = super::menu_rules::collect_menu_action_ids(&self.menu.items)?;
        let catalog_executable_action_ids =
            super::action_rules::validate_action_catalog(&self.actions)?;
        let executable_action_ids = if self.actions.is_empty() {
            &menu_action_ids.executable
        } else {
            &catalog_executable_action_ids
        };
        super::runtime_rules::validate_optional_runtime_config(
            self.runtime.as_ref(),
            executable_action_ids,
            !self.actions.is_empty(),
        )?;
        super::shortcut_rules::validate_shortcuts(&self.shortcuts, executable_action_ids)?;
        super::command_rules::validate_optional_daemon_config(self.daemon.as_ref())?;
        super::action_rules::validate_continuous_action_transport(
            &self.actions,
            self.daemon.as_ref(),
        )?;
        super::dependency_rules::validate_optional_dependencies(self.dependencies.as_ref())?;
        Ok(())
    }

    pub fn validate_version(&self) -> Result<()> {
        validate_manifest_version(self.manifest_version)
    }

    pub fn read_from_dir(dir: impl AsRef<std::path::Path>) -> Result<Self> {
        let path = dir.as_ref().join("plugin.toml");
        let raw =
            std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
        toml::from_str(&raw).with_context(|| format!("parse {}", path.display()))
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

    const SAMPLE_MANIFEST: &str =
        "[plugin]\nid = \"plugin-x\"\nname = \"X\"\ndescription = \"\"\nversion = \"1.2.3\"\n[menu]\nlabel = \"\"\nitems = []\n";

    #[test]
    fn read_from_dir_parses_manifest_in_dir() {
        let dir = std::env::temp_dir().join("qol-plugin-api-read-from-dir-ok");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("plugin.toml"), SAMPLE_MANIFEST).unwrap();

        let manifest = PluginManifest::read_from_dir(&dir).unwrap();

        assert_eq!(manifest.plugin.name, "X");
        assert_eq!(manifest.plugin.version, "1.2.3");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn read_from_dir_errors_when_manifest_missing() {
        let dir = std::env::temp_dir().join("qol-plugin-api-read-from-dir-missing");
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();

        assert!(PluginManifest::read_from_dir(&dir).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }
}
