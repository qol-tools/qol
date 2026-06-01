pub mod zigbee;

use anyhow::Result;

use crate::domain::model::{BackendHealth, LightCommand, LightTarget, LightTargetInfo};

pub trait LightBackend {
    fn kind(&self) -> &'static str;
    fn health(&self) -> BackendHealth;
    fn list_targets(&self) -> Result<Vec<LightTargetInfo>>;
    fn apply_command(&mut self, target: &LightTarget, command: &LightCommand) -> Result<()>;
}
