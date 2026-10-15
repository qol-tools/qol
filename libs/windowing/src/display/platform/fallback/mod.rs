use super::DisplayEnumerator;
use crate::display::{
    DisplayError, DisplayHandle, DisplayMode, DisplayOps, DisplayPlacement, DisplaySnapshot,
};

pub struct Platform;

impl DisplayEnumerator for Platform {
    fn enumerate(&self) -> Result<Vec<DisplayHandle>, DisplayError> {
        Err(DisplayError::UnsupportedPlatform)
    }

    fn snapshot(&self) -> Result<Vec<DisplaySnapshot>, DisplayError> {
        Err(DisplayError::Unsupported {
            capability: "snapshot",
            reason: "display snapshots are not implemented on this platform".into(),
        })
    }
}

impl DisplayOps for Platform {
    fn modes(&self, _handle: &DisplayHandle) -> Result<Vec<DisplayMode>, DisplayError> {
        Err(DisplayError::Unsupported {
            capability: "modes",
            reason: "mode control is not implemented on this platform".into(),
        })
    }

    fn set_mode(&self, _handle: &DisplayHandle, _mode: &DisplayMode) -> Result<(), DisplayError> {
        Err(DisplayError::Unsupported {
            capability: "modes",
            reason: "mode control is not implemented on this platform".into(),
        })
    }

    fn set_layout(&self, _placements: &[DisplayPlacement]) -> Result<(), DisplayError> {
        Err(DisplayError::Unsupported {
            capability: "layout",
            reason: "display layout control is not implemented on this platform".into(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stub_returns_typed_error() {
        let error = Platform.enumerate().unwrap_err();
        assert!(matches!(error, DisplayError::UnsupportedPlatform));
    }

    #[test]
    fn snapshot_and_ops_return_unsupported() {
        assert!(matches!(
            Platform.snapshot(),
            Err(DisplayError::Unsupported { .. })
        ));
        let handle = DisplayHandle::new("id-1".into(), "card0-DP-1".into(), None, false);
        assert!(matches!(
            Platform.modes(&handle),
            Err(DisplayError::Unsupported { .. })
        ));
        let mode = DisplayMode {
            token: 1,
            width: 1920,
            height: 1080,
            refresh_hz: 60,
        };
        assert!(matches!(
            Platform.set_mode(&handle, &mode),
            Err(DisplayError::Unsupported { .. })
        ));
        let placements = vec![DisplayPlacement {
            handle,
            x: 0,
            y: 0,
            primary: true,
        }];
        assert!(matches!(
            Platform.set_layout(&placements),
            Err(DisplayError::Unsupported { .. })
        ));
    }
}
