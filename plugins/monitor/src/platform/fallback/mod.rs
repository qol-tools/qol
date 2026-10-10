use std::sync::mpsc::Sender;
use std::sync::Arc;

use super::{Control, PlatformSupport};
use crate::daemon::Command;
use crate::monitor::StubControl;

pub(crate) struct SignalGuard;

pub(crate) fn current_support() -> PlatformSupport {
    PlatformSupport {
        name: std::env::consts::OS,
        supported: false,
    }
}

pub(crate) fn control() -> Control {
    Arc::new(StubControl)
}

pub(crate) fn native_desktop() -> bool {
    false
}

pub(crate) fn install_signal_handlers(_tx: Sender<Command>) -> SignalGuard {
    SignalGuard
}
