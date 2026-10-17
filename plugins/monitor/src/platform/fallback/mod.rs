use std::sync::mpsc::Sender;
use std::sync::Arc;

use super::{Control, PlatformSupport};
use crate::daemon::Command;
use crate::monitor::night::Tint;
use crate::monitor::{GammaTable, StubControl};

pub(crate) struct SignalGuard;

pub(crate) fn current_support() -> PlatformSupport {
    PlatformSupport {
        name: qol_platform::os::current_name(),
        supported: false,
    }
}

pub(crate) fn control() -> Control {
    Arc::new(StubControl)
}

pub(crate) fn native_desktop() -> bool {
    false
}

pub(crate) fn fit_tint(_dimmed: &GammaTable, tint: Tint) -> Tint {
    tint
}

pub(crate) fn brightness_floor(_base: &GammaTable) -> u8 {
    crate::monitor::BRIGHTNESS_MIN
}

pub(crate) fn install_signal_handlers(_tx: Sender<Command>) -> SignalGuard {
    SignalGuard
}
