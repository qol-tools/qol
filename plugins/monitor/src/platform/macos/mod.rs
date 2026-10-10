use std::sync::Arc;

mod core_graphics;
mod iokit;

use super::platform_control::PlatformControl;
use super::{Control, PlatformSupport};
use crate::monitor::backends::avservice::MacAvServiceBackend;
use crate::monitor::backends::cg_gamma::CgGammaControl;
use crate::monitor::backends::shared_display::SharedDisplay;
use crate::monitor::PolicyControl;
use core_graphics::{display_identity, CoreGraphicsSeam};
use iokit::IokitAvTransport;

pub(crate) use super::unix_signals::install_signal_handlers;

pub(crate) fn current_support() -> PlatformSupport {
    PlatformSupport {
        name: "macos",
        supported: true,
    }
}

pub(crate) fn control() -> Control {
    Arc::new(PlatformControl::new(
        PolicyControl::new(
            MacAvServiceBackend::new(IokitAvTransport::new(), Box::new(display_identity)),
            CgGammaControl::new(CoreGraphicsSeam),
        ),
        SharedDisplay::new(),
        mode_writes_supported,
    ))
}

pub(crate) fn native_desktop() -> bool {
    false
}

fn mode_writes_supported() -> bool {
    false
}
