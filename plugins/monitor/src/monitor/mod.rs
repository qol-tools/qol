use std::fmt;

use qol_windowing::display::{DisplayError, DisplayHandle};
use qol_windowing::DisplayEnumerator;

pub const BRIGHTNESS_MIN: u8 = 0;
pub const BRIGHTNESS_MAX: u8 = 100;
pub const BRIGHTNESS_STEP: u8 = 5;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DisplayCapabilities {
    pub brightness_ddc: bool,
    pub brightness_gamma: bool,
    pub contrast: bool,
    pub modes: bool,
    pub hdr: bool,
}

impl DisplayCapabilities {
    pub fn none() -> Self {
        Self::default()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrightnessSource {
    Ddc,
    Gamma,
}

impl BrightnessSource {
    pub fn label(self) -> &'static str {
        match self {
            Self::Ddc => "ddc",
            Self::Gamma => "gamma",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BrightnessState {
    pub value: u8,
    pub source: BrightnessSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GammaState {
    pub value: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayMode {
    pub width: u32,
    pub height: u32,
    pub refresh_hz: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HdrState {
    pub enabled: bool,
}

#[derive(Debug)]
pub enum MonitorError {
    Unsupported {
        capability: &'static str,
        reason: &'static str,
    },
    DisplayNotFound(String),
    Display(DisplayError),
}

impl MonitorError {
    pub fn unsupported(capability: &'static str, reason: &'static str) -> Self {
        Self::Unsupported { capability, reason }
    }
}

impl fmt::Display for MonitorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported { capability, reason } => {
                write!(f, "{capability} control is not implemented yet: {reason}")
            }
            Self::DisplayNotFound(selector) => {
                write!(f, "no display matches `{selector}`")
            }
            Self::Display(error) => write!(f, "display enumeration failed: {error}"),
        }
    }
}

impl std::error::Error for MonitorError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Display(error) => Some(error),
            Self::Unsupported { .. } | Self::DisplayNotFound(_) => None,
        }
    }
}

impl From<DisplayError> for MonitorError {
    fn from(error: DisplayError) -> Self {
        Self::Display(error)
    }
}

pub trait DisplayControl: Send + Sync {
    fn enumerate(&self) -> Result<Vec<DisplayHandle>, MonitorError>;
    fn probe(&self, handle: &DisplayHandle) -> Result<DisplayCapabilities, MonitorError>;
    fn get_brightness(&self, handle: &DisplayHandle) -> Result<BrightnessState, MonitorError>;
    fn set_brightness(&self, handle: &DisplayHandle, value: u8) -> Result<(), MonitorError>;
    fn get_gamma(&self, handle: &DisplayHandle) -> Result<GammaState, MonitorError>;
    fn set_gamma(&self, handle: &DisplayHandle, value: u8) -> Result<(), MonitorError>;
    fn list_modes(&self, handle: &DisplayHandle) -> Result<Vec<DisplayMode>, MonitorError>;
    fn set_mode(&self, handle: &DisplayHandle, mode: &DisplayMode) -> Result<(), MonitorError>;
    fn get_hdr(&self, handle: &DisplayHandle) -> Result<HdrState, MonitorError>;
    fn set_hdr(&self, handle: &DisplayHandle, enabled: bool) -> Result<(), MonitorError>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct StubControl;

const BRIGHTNESS_REASON: &str = "the DDC and gamma backends land in later phases";
const GAMMA_REASON: &str = "the gamma fallback lands in a later phase";
const MODES_REASON: &str = "mode control lands in a later phase";
const HDR_REASON: &str = "HDR control lands in a later phase";

impl DisplayControl for StubControl {
    fn enumerate(&self) -> Result<Vec<DisplayHandle>, MonitorError> {
        Ok(qol_windowing::Platform.enumerate()?)
    }

    fn probe(&self, _handle: &DisplayHandle) -> Result<DisplayCapabilities, MonitorError> {
        Ok(DisplayCapabilities::none())
    }

    fn get_brightness(&self, _handle: &DisplayHandle) -> Result<BrightnessState, MonitorError> {
        Err(MonitorError::unsupported("brightness", BRIGHTNESS_REASON))
    }

    fn set_brightness(&self, _handle: &DisplayHandle, _value: u8) -> Result<(), MonitorError> {
        Err(MonitorError::unsupported("brightness", BRIGHTNESS_REASON))
    }

    fn get_gamma(&self, _handle: &DisplayHandle) -> Result<GammaState, MonitorError> {
        Err(MonitorError::unsupported("gamma", GAMMA_REASON))
    }

    fn set_gamma(&self, _handle: &DisplayHandle, _value: u8) -> Result<(), MonitorError> {
        Err(MonitorError::unsupported("gamma", GAMMA_REASON))
    }

    fn list_modes(&self, _handle: &DisplayHandle) -> Result<Vec<DisplayMode>, MonitorError> {
        Err(MonitorError::unsupported("modes", MODES_REASON))
    }

    fn set_mode(&self, _handle: &DisplayHandle, _mode: &DisplayMode) -> Result<(), MonitorError> {
        Err(MonitorError::unsupported("modes", MODES_REASON))
    }

    fn get_hdr(&self, _handle: &DisplayHandle) -> Result<HdrState, MonitorError> {
        Err(MonitorError::unsupported("hdr", HDR_REASON))
    }

    fn set_hdr(&self, _handle: &DisplayHandle, _enabled: bool) -> Result<(), MonitorError> {
        Err(MonitorError::unsupported("hdr", HDR_REASON))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn handle() -> DisplayHandle {
        DisplayHandle::new("id-1".into(), "card0-DP-1".into(), None, false)
    }

    #[test]
    fn stub_brightness_returns_typed_unsupported() {
        let error = StubControl.get_brightness(&handle()).unwrap_err();
        match error {
            MonitorError::Unsupported { capability, reason } => {
                assert_eq!(capability, "brightness");
                assert!(!reason.is_empty());
            }
            other => panic!("expected Unsupported, got {other:?}"),
        }
    }

    #[test]
    fn stub_control_stubs_every_capability() {
        let control = StubControl;
        let handle = handle();
        assert!(matches!(
            control.set_brightness(&handle, 50),
            Err(MonitorError::Unsupported {
                capability: "brightness",
                ..
            })
        ));
        assert!(matches!(
            control.get_gamma(&handle),
            Err(MonitorError::Unsupported {
                capability: "gamma",
                ..
            })
        ));
        assert!(matches!(
            control.set_gamma(&handle, 50),
            Err(MonitorError::Unsupported {
                capability: "gamma",
                ..
            })
        ));
        assert!(matches!(
            control.list_modes(&handle),
            Err(MonitorError::Unsupported {
                capability: "modes",
                ..
            })
        ));
        assert!(matches!(
            control.set_mode(
                &handle,
                &DisplayMode {
                    width: 1920,
                    height: 1080,
                    refresh_hz: 60
                }
            ),
            Err(MonitorError::Unsupported {
                capability: "modes",
                ..
            })
        ));
        assert!(matches!(
            control.get_hdr(&handle),
            Err(MonitorError::Unsupported {
                capability: "hdr",
                ..
            })
        ));
        assert!(matches!(
            control.set_hdr(&handle, true),
            Err(MonitorError::Unsupported {
                capability: "hdr",
                ..
            })
        ));
    }

    #[test]
    fn stub_probe_reports_no_capabilities_yet() {
        assert_eq!(
            StubControl.probe(&handle()).unwrap(),
            DisplayCapabilities::none()
        );
    }

    #[test]
    fn unsupported_error_renders_the_capability_and_reason() {
        let error = MonitorError::unsupported("modes", MODES_REASON);
        assert_eq!(
            error.to_string(),
            "modes control is not implemented yet: mode control lands in a later phase"
        );
    }

    #[test]
    fn brightness_source_labels_are_hardware_and_software() {
        assert_eq!(BrightnessSource::Ddc.label(), "ddc");
        assert_eq!(BrightnessSource::Gamma.label(), "gamma");
    }
}
