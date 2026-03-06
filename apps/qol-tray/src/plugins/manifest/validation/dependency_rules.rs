use super::{BinaryDependency, Dependencies};
use anyhow::Result;

impl Dependencies {
    pub fn validate(&self) -> Result<()> {
        for binary in &self.binaries {
            binary.validate()?;
        }
        Ok(())
    }
}

impl BinaryDependency {
    pub fn validate(&self) -> Result<()> {
        super::command_rules::validate_command_name("dependencies.binaries.name", &self.name)
    }
}

pub(super) fn validate_optional_dependencies(dependencies: Option<&Dependencies>) -> Result<()> {
    let Some(dependencies) = dependencies else {
        return Ok(());
    };

    dependencies.validate()
}
