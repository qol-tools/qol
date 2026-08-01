pub mod zigbee;

use anyhow::Result;

use crate::domain::model::{LightCommand, LightTarget};

pub trait LightBackend {
    fn apply_command(&mut self, target: &LightTarget, command: &LightCommand) -> Result<()>;
}
