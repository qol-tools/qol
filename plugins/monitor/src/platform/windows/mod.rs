use std::sync::Arc;

mod display;
mod dxva2;
mod gamma;
mod signals;

use super::platform_control::PlatformControl;
use super::{Control, PlatformSupport};
use crate::monitor::backends::shared_display::SharedDisplay;
use crate::monitor::backends::vcp_ddc::VcpDdcBackend;
use crate::monitor::{GammaBackend, PolicyControl};
use display::WindowsDisplay;
use dxva2::Dxva2Transport;
use gamma::GdiGammaTransport;

pub(crate) use gamma::{brightness_floor, fit_tint};
pub(crate) use signals::install_signal_handlers;

pub(crate) fn current_support() -> PlatformSupport {
    PlatformSupport {
        name: qol_platform::Os::Windows.name(),
        supported: true,
    }
}

pub(crate) fn control() -> Control {
    Arc::new(PlatformControl::new(
        PolicyControl::new(
            VcpDdcBackend::new(Dxva2Transport, WindowsDisplay),
            GammaBackend::new(GdiGammaTransport),
        ),
        SharedDisplay::with_platform(WindowsDisplay),
        mode_writes_supported,
    ))
}

pub(crate) fn native_desktop() -> bool {
    true
}

fn mode_writes_supported() -> bool {
    true
}
