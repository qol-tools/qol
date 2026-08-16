use std::sync::Arc;

use super::{Control, PlatformSupport};
use crate::monitor::backends::i2c_ddc::{I2cDdcBackend, LinuxI2cTransport};
use crate::monitor::backends::x11_randr_gamma::X11GammaTransport;
use crate::monitor::{GammaBackend, PolicyControl};

pub(crate) fn current_support() -> PlatformSupport {
    PlatformSupport {
        name: "linux",
        supported: true,
    }
}

pub(crate) fn control() -> Control {
    Arc::new(PolicyControl::new(
        I2cDdcBackend::new(LinuxI2cTransport),
        GammaBackend::new(X11GammaTransport),
    ))
}
