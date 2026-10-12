mod platform;

use anyhow::Result;
use std::path::{Path, PathBuf};

#[derive(Debug, PartialEq, Eq)]
pub enum AutostartState {
    Enabled(PathBuf),
    DisabledByUser,
    Missing,
}

impl AutostartState {
    pub(crate) fn from_target(target: Option<PathBuf>) -> Self {
        target.map_or(Self::Missing, Self::Enabled)
    }
}

pub fn read_target() -> Result<Option<PathBuf>> {
    Ok(match platform::read_state()? {
        AutostartState::Enabled(target) => Some(target),
        AutostartState::DisabledByUser | AutostartState::Missing => None,
    })
}

pub fn read_state() -> Result<AutostartState> {
    platform::read_state()
}

pub fn write_target(binary: &Path) -> Result<()> {
    platform::write_target(binary)
}

pub fn autostart_path() -> Result<PathBuf> {
    platform::autostart_path()
}
