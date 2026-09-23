use std::sync::Arc;

use qol_windowing::display::{
    DisplayEnumerator, DisplayError, DisplayHandle, DisplayMode, DisplayOps, DisplayPlacement,
    DisplaySnapshot,
};

use super::{Control, DisplayServer, PlatformControl, PlatformSupport};
use crate::monitor::backends::i2c_ddc::{I2cDdcBackend, LinuxI2cTransport};
use crate::monitor::backends::shared_display::SharedDisplay;
use crate::monitor::backends::x11_randr_gamma::X11GammaTransport;
use crate::monitor::{GammaBackend, PolicyControl};

pub(crate) fn current_support() -> PlatformSupport {
    PlatformSupport {
        name: "linux",
        supported: true,
    }
}

pub(crate) fn control() -> Control {
    Arc::new(PlatformControl::new(
        PolicyControl::new(
            I2cDdcBackend::new(LinuxI2cTransport),
            GammaBackend::new(X11GammaTransport),
        ),
        SharedDisplay::with_platform(X11Display {
            platform: qol_windowing::Platform,
        }),
    ))
}

struct X11Display {
    platform: qol_windowing::Platform,
}

impl DisplayEnumerator for X11Display {
    fn enumerate(&self) -> Result<Vec<DisplayHandle>, DisplayError> {
        self.platform.enumerate()
    }

    fn snapshot(&self) -> Result<Vec<DisplaySnapshot>, DisplayError> {
        gate(super::display_server(), "displays")?;
        self.platform.snapshot()
    }
}

impl DisplayOps for X11Display {
    fn modes(&self, handle: &DisplayHandle) -> Result<Vec<DisplayMode>, DisplayError> {
        gate(super::display_server(), "modes")?;
        self.platform.modes(handle)
    }

    fn set_mode(&self, handle: &DisplayHandle, mode: &DisplayMode) -> Result<(), DisplayError> {
        gate(super::display_server(), "modes")?;
        self.platform.set_mode(handle, mode)
    }

    fn set_layout(&self, placements: &[DisplayPlacement]) -> Result<(), DisplayError> {
        gate(super::display_server(), "layout")?;
        self.platform.set_layout(placements)
    }
}

fn gate(server: DisplayServer, capability: &'static str) -> Result<(), DisplayError> {
    if server == DisplayServer::X11 {
        Ok(())
    } else {
        Err(DisplayError::Unsupported {
            capability,
            reason: "display configuration needs an X11 session".to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wayland_or_headless_session_refuses_display_writes() {
        for server in [DisplayServer::Wayland, DisplayServer::None] {
            match gate(server, "layout").unwrap_err() {
                DisplayError::Unsupported { capability, reason } => {
                    assert_eq!(capability, "layout");
                    assert_eq!(reason, "display configuration needs an X11 session");
                }
                other => panic!("expected Unsupported, got {other:?}"),
            }
        }
        assert!(gate(DisplayServer::X11, "modes").is_ok());
    }

    #[test]
    fn a_wayland_or_headless_session_refuses_display_reads() {
        for server in [DisplayServer::Wayland, DisplayServer::None] {
            for capability in ["displays", "modes"] {
                match gate(server, capability).unwrap_err() {
                    DisplayError::Unsupported {
                        capability: refused,
                        reason,
                    } => {
                        assert_eq!(refused, capability);
                        assert_eq!(reason, "display configuration needs an X11 session");
                    }
                    other => panic!("expected Unsupported, got {other:?}"),
                }
            }
        }
        assert!(gate(DisplayServer::X11, "displays").is_ok());
        assert!(gate(DisplayServer::X11, "modes").is_ok());
    }
}
