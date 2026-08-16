use std::sync::Arc;

use super::{Control, PlatformSupport};
use crate::monitor::backends::avservice::{
    IokitAvTransport, MacAvServiceBackend, UnsupportedGamma,
};
use crate::monitor::PolicyControl;

pub(crate) fn current_support() -> PlatformSupport {
    PlatformSupport {
        name: "macos",
        supported: true,
    }
}

pub(crate) fn control() -> Control {
    Arc::new(PolicyControl::new(
        MacAvServiceBackend::new(IokitAvTransport::new()),
        UnsupportedGamma,
    ))
}
