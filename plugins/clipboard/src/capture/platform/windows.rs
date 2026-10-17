use std::sync::Arc;

use crate::capture::{CaptureError, Clipboard, Support};

pub(crate) fn open() -> Result<Arc<dyn Clipboard>, CaptureError> {
    Err(unsupported())
}

pub(crate) fn probe() -> Support {
    Support::Unavailable(unsupported())
}

fn unsupported() -> CaptureError {
    CaptureError::Unsupported {
        platform: qol_platform::Os::Windows.label(),
    }
}
