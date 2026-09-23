use std::sync::Arc;

use super::{Control, PlatformControl, PlatformSupport};
use crate::monitor::backends::avservice::{IokitAvTransport, MacAvServiceBackend};
use crate::monitor::backends::cg_gamma::{CgGammaControl, CoreGraphicsSeam};
use crate::monitor::backends::shared_display::SharedDisplay;
use crate::monitor::PolicyControl;

pub(crate) fn current_support() -> PlatformSupport {
    PlatformSupport {
        name: "macos",
        supported: true,
    }
}

pub(crate) fn control() -> Control {
    Arc::new(PlatformControl::new(
        PolicyControl::new(
            MacAvServiceBackend::new(IokitAvTransport::new()),
            CgGammaControl::new(CoreGraphicsSeam),
        ),
        SharedDisplay::new(),
    ))
}
