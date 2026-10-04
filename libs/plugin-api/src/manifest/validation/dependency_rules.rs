use crate::manifest::{BinaryDependency, Dependencies, SystemDependency};
use anyhow::{bail, Result};

const MANIFEST_PLATFORMS: [&str; 3] = ["linux", "macos", "windows"];

impl Dependencies {
    pub fn validate(&self) -> Result<()> {
        for binary in &self.binaries {
            binary.validate()?;
        }
        for system in &self.system {
            system.validate()?;
        }
        Ok(())
    }
}

impl BinaryDependency {
    pub fn validate(&self) -> Result<()> {
        super::command_rules::validate_command_name("dependencies.binaries.name", &self.name)
    }
}

impl SystemDependency {
    pub fn validate(&self) -> Result<()> {
        for (field, value) in [
            ("name", &self.name),
            ("license", &self.license),
            ("url", &self.url),
        ] {
            if value.trim().is_empty() {
                bail!("dependencies.system.{field} must not be empty");
            }
        }
        if self.platforms.is_empty() {
            bail!("dependencies.system.platforms must name at least one platform");
        }
        if let Some(unknown) = self
            .platforms
            .iter()
            .find(|platform| !MANIFEST_PLATFORMS.contains(&platform.as_str()))
        {
            bail!("dependencies.system.platforms has unknown platform {unknown:?}");
        }
        if parse_version(&self.min_version).is_none() {
            bail!(
                "dependencies.system.min_version {:?} is not MAJOR.MINOR.PATCH",
                self.min_version
            );
        }
        Ok(())
    }
}

pub fn parse_version(raw: &str) -> Option<(u64, u64, u64)> {
    let mut parts = raw.split('.').map(|part| part.parse::<u64>().ok());
    let version = (parts.next()??, parts.next()??, parts.next()??);
    parts.next().is_none().then_some(version)
}

pub(super) fn validate_optional_dependencies(dependencies: Option<&Dependencies>) -> Result<()> {
    let Some(dependencies) = dependencies else {
        return Ok(());
    };

    dependencies.validate()
}
