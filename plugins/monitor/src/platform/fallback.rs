use std::sync::Arc;

use super::{Control, PlatformSupport};
use crate::monitor::StubControl;

pub(crate) fn current_support() -> PlatformSupport {
    PlatformSupport {
        name: std::env::consts::OS,
        supported: false,
    }
}

pub(crate) fn control() -> Control {
    Arc::new(StubControl)
}
