use std::fmt;
use std::path::Path;
use std::sync::Arc;

use crate::session::RestoreMode;

#[cfg(target_os = "linux")]
mod linux;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TakeoverOutcome {
    Disabled,
    AlreadyOff,
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostNightLightStatus {
    TakenOver,
    Off,
    Unsupported,
    Failed,
}

impl HostNightLightStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::TakenOver => "taken_over",
            Self::Off => "off",
            Self::Unsupported => "unsupported",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostNightLightError {
    Unsupported(String),
    Failed(String),
}

impl fmt::Display for HostNightLightError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported(reason) => write!(formatter, "{reason}"),
            Self::Failed(reason) => write!(formatter, "{reason}"),
        }
    }
}

impl std::error::Error for HostNightLightError {}

pub trait HostNightLight: Send + Sync {
    fn take_over(&self) -> Result<TakeoverOutcome, HostNightLightError>;
    fn release(&self, mode: RestoreMode) -> Result<(), HostNightLightError>;
    fn mark_handoff(&self, successor: Option<&str>);
    fn is_taken_over(&self) -> bool;
    fn status(&self) -> HostNightLightStatus;
}

#[derive(Default)]
pub struct NoopHostNightLight;

impl HostNightLight for NoopHostNightLight {
    fn take_over(&self) -> Result<TakeoverOutcome, HostNightLightError> {
        Ok(TakeoverOutcome::Unsupported)
    }

    fn release(&self, _mode: RestoreMode) -> Result<(), HostNightLightError> {
        Ok(())
    }

    fn mark_handoff(&self, _successor: Option<&str>) {}

    fn is_taken_over(&self) -> bool {
        false
    }

    fn status(&self) -> HostNightLightStatus {
        HostNightLightStatus::Unsupported
    }
}

pub fn control(config_root: Option<&Path>) -> Arc<dyn HostNightLight> {
    #[cfg(target_os = "linux")]
    {
        linux::control(config_root)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = config_root;
        Arc::new(NoopHostNightLight)
    }
}
