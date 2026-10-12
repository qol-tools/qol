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

    pub fn into_target(self) -> Option<PathBuf> {
        match self {
            Self::Enabled(target) => Some(target),
            Self::DisabledByUser | Self::Missing => None,
        }
    }
}

pub fn read_target() -> Result<Option<PathBuf>> {
    platform::read_state().map(AutostartState::into_target)
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
