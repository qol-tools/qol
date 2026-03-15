use anyhow::{anyhow, Result};

use crate::backend::LightBackend;
use crate::domain::model::{BackendHealth, LightCommand, LightTarget, LightTargetInfo};

pub struct LightService<B> {
    backend: B,
}

impl<B: LightBackend> LightService<B> {
    pub fn new(backend: B) -> Self {
        Self { backend }
    }

    pub fn health(&self) -> BackendHealth {
        self.backend.health()
    }

    pub fn list_targets(&self) -> Result<Vec<LightTargetInfo>> {
        self.backend.list_targets()
    }

    pub fn backend(&self) -> &B {
        &self.backend
    }

    pub fn apply_command(&mut self, target: &LightTarget, command: &LightCommand) -> Result<()> {
        validate_command(command)?;
        self.backend.apply_command(target, command)
    }
}

fn validate_command(command: &LightCommand) -> Result<()> {
    if let LightCommand::SetBrightness { level } = command {
        if *level > 100 {
            return Err(anyhow!("brightness must be between 0 and 100"));
        }
    }
    if let LightCommand::SetColorTemperature { mirek } = command {
        if *mirek == 0 {
            return Err(anyhow!("color temperature must be greater than 0"));
        }
    }
    Ok(())
}
