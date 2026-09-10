use qol_windowing::display::{
    DisplayEnumerator, DisplayError, DisplayHandle, DisplayMode, DisplayOps, DisplayPlacement,
    DisplaySnapshot,
};

use crate::monitor::MonitorError;

pub struct SharedDisplay<P = qol_windowing::Platform> {
    platform: P,
}

impl SharedDisplay<qol_windowing::Platform> {
    pub fn new() -> Self {
        Self {
            platform: qol_windowing::Platform,
        }
    }
}

impl Default for SharedDisplay<qol_windowing::Platform> {
    fn default() -> Self {
        Self::new()
    }
}

impl<P> SharedDisplay<P> {
    pub fn with_platform(platform: P) -> Self {
        Self { platform }
    }
}

impl<P: DisplayEnumerator + DisplayOps> SharedDisplay<P> {
    pub fn snapshot(&self) -> Result<Vec<DisplaySnapshot>, MonitorError> {
        self.platform
            .snapshot()
            .map_err(|error| map_error(error, "displays"))
    }

    pub fn list_modes(&self, handle: &DisplayHandle) -> Result<Vec<DisplayMode>, MonitorError> {
        self.platform
            .modes(handle)
            .map_err(|error| map_error(error, "modes"))
    }

    pub fn set_mode(&self, handle: &DisplayHandle, mode: &DisplayMode) -> Result<(), MonitorError> {
        self.platform
            .set_mode(handle, mode)
            .map_err(|error| map_error(error, "modes"))
    }

    pub fn set_layout(&self, placements: &[DisplayPlacement]) -> Result<(), MonitorError> {
        self.platform
            .set_layout(placements)
            .map_err(|error| map_error(error, "layout"))
    }
}

fn map_error(error: DisplayError, operation: &'static str) -> MonitorError {
    match error {
        DisplayError::Unsupported { capability, reason } => {
            MonitorError::unsupported(capability, reason)
        }
        DisplayError::UnsupportedPlatform => {
            MonitorError::unsupported(operation, DisplayError::UnsupportedPlatform.to_string())
        }
        DisplayError::NotFound { selector, .. } => MonitorError::DisplayNotFound(selector),
        DisplayError::LayoutInvalid { reason } => MonitorError::refused(operation, reason),
        DisplayError::Io(error) => MonitorError::Display(DisplayError::Io(error)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qol_windowing::MonitorBounds;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Failure {
        Unsupported,
        UnsupportedPlatform,
        NotFound,
        LayoutInvalid,
        Io,
    }

    fn failure_error(failure: Failure) -> DisplayError {
        match failure {
            Failure::Unsupported => DisplayError::Unsupported {
                capability: "modes",
                reason: "the server answered no modes".to_string(),
            },
            Failure::UnsupportedPlatform => DisplayError::UnsupportedPlatform,
            Failure::NotFound => DisplayError::NotFound {
                capability: "modes",
                selector: "card0-DP-1".to_string(),
            },
            Failure::LayoutInvalid => DisplayError::LayoutInvalid {
                reason: "the write did not verify".to_string(),
            },
            Failure::Io => DisplayError::Io(std::io::Error::other("the bus closed")),
        }
    }

    #[derive(Clone)]
    struct FakePlatform {
        failure: Option<Failure>,
        calls: Arc<Mutex<Vec<String>>>,
    }

    impl FakePlatform {
        fn answering() -> Self {
            Self {
                failure: None,
                calls: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn failing(failure: Failure) -> Self {
            Self {
                failure: Some(failure),
                calls: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }

        fn record(&self, call: String) {
            self.calls.lock().unwrap().push(call);
        }

        fn result<T>(&self, value: T) -> Result<T, DisplayError> {
            match self.failure {
                Some(failure) => Err(failure_error(failure)),
                None => Ok(value),
            }
        }
    }

    impl DisplayEnumerator for FakePlatform {
        fn enumerate(&self) -> Result<Vec<DisplayHandle>, DisplayError> {
            self.record("enumerate".to_string());
            self.result(vec![handle()])
        }

        fn snapshot(&self) -> Result<Vec<DisplaySnapshot>, DisplayError> {
            self.record("snapshot".to_string());
            self.result(vec![DisplaySnapshot {
                handle: handle(),
                bounds: MonitorBounds {
                    x: 0.0,
                    y: 0.0,
                    width: 1920.0,
                    height: 1080.0,
                },
                primary: true,
                mode: Some(mode()),
            }])
        }
    }

    impl DisplayOps for FakePlatform {
        fn modes(&self, handle: &DisplayHandle) -> Result<Vec<DisplayMode>, DisplayError> {
            self.record(format!("modes:{}:{}", handle.id(), handle.connector()));
            self.result(vec![mode()])
        }

        fn set_mode(&self, handle: &DisplayHandle, mode: &DisplayMode) -> Result<(), DisplayError> {
            self.record(format!(
                "set_mode:{}:{}:{}x{}@{}:{}",
                handle.id(),
                handle.connector(),
                mode.width,
                mode.height,
                mode.refresh_hz,
                mode.token
            ));
            self.result(())
        }

        fn set_layout(&self, placements: &[DisplayPlacement]) -> Result<(), DisplayError> {
            let described = placements
                .iter()
                .map(|placement| {
                    format!(
                        "{}:{},{}:primary={}:{}",
                        placement.handle.connector(),
                        placement.x,
                        placement.y,
                        placement.primary,
                        placement.handle.id()
                    )
                })
                .collect::<Vec<_>>()
                .join(";");
            self.record(format!("set_layout:{described}"));
            self.result(())
        }
    }

    fn handle() -> DisplayHandle {
        DisplayHandle::new("id-1".to_string(), "card0-DP-1".to_string(), None, false)
    }

    fn mode() -> DisplayMode {
        DisplayMode {
            token: 9,
            width: 1920,
            height: 1080,
            refresh_hz: 60,
        }
    }

    fn placements() -> Vec<DisplayPlacement> {
        vec![DisplayPlacement {
            handle: handle(),
            x: -1920,
            y: 40,
            primary: true,
        }]
    }

    #[test]
    fn shared_display_forwards_reads_and_writes_to_the_platform() {
        let platform = FakePlatform::answering();
        let display = SharedDisplay::with_platform(platform.clone());
        let snapshots = display.snapshot().unwrap();
        assert_eq!(snapshots.len(), 1);
        assert_eq!(snapshots[0].handle.id(), "id-1");
        assert_eq!(display.list_modes(&handle()).unwrap(), vec![mode()]);
        display.set_mode(&handle(), &mode()).unwrap();
        display.set_layout(&placements()).unwrap();
        assert_eq!(
            platform.calls(),
            vec![
                "snapshot".to_string(),
                "modes:id-1:card0-DP-1".to_string(),
                "set_mode:id-1:card0-DP-1:1920x1080@60:9".to_string(),
                "set_layout:card0-DP-1:-1920,40:primary=true:id-1".to_string(),
            ]
        );
    }

    #[test]
    fn unsupported_platform_uses_the_operation_capability() {
        let failure = Failure::UnsupportedPlatform;
        let display = SharedDisplay::with_platform(FakePlatform::failing(failure));
        match display.snapshot().unwrap_err() {
            MonitorError::Unsupported { capability, .. } => assert_eq!(capability, "displays"),
            other => panic!("expected Unsupported, got {other:?}"),
        }
        match display.list_modes(&handle()).unwrap_err() {
            MonitorError::Unsupported { capability, .. } => assert_eq!(capability, "modes"),
            other => panic!("expected Unsupported, got {other:?}"),
        }
        match display.set_mode(&handle(), &mode()).unwrap_err() {
            MonitorError::Unsupported { capability, .. } => assert_eq!(capability, "modes"),
            other => panic!("expected Unsupported, got {other:?}"),
        }
        match display.set_layout(&[]).unwrap_err() {
            MonitorError::Unsupported { capability, .. } => assert_eq!(capability, "layout"),
            other => panic!("expected Unsupported, got {other:?}"),
        }
    }

    #[test]
    fn unsupported_capability_is_kept_from_the_platform_error() {
        let display = SharedDisplay::with_platform(FakePlatform::failing(Failure::Unsupported));
        match display.list_modes(&handle()).unwrap_err() {
            MonitorError::Unsupported { capability, reason } => {
                assert_eq!(capability, "modes");
                assert_eq!(reason, "the server answered no modes");
            }
            other => panic!("expected Unsupported, got {other:?}"),
        }
    }

    #[test]
    fn not_found_becomes_a_display_not_found() {
        let display = SharedDisplay::with_platform(FakePlatform::failing(Failure::NotFound));
        match display.list_modes(&handle()).unwrap_err() {
            MonitorError::DisplayNotFound(selector) => assert_eq!(selector, "card0-DP-1"),
            other => panic!("expected DisplayNotFound, got {other:?}"),
        }
        match display.set_mode(&handle(), &mode()).unwrap_err() {
            MonitorError::DisplayNotFound(selector) => assert_eq!(selector, "card0-DP-1"),
            other => panic!("expected DisplayNotFound, got {other:?}"),
        }
    }

    #[test]
    fn layout_invalid_uses_the_operation_capability() {
        let display = SharedDisplay::with_platform(FakePlatform::failing(Failure::LayoutInvalid));
        match display.list_modes(&handle()).unwrap_err() {
            MonitorError::Refused { capability, reason } => {
                assert_eq!(capability, "modes");
                assert_eq!(reason, "the write did not verify");
            }
            other => panic!("expected Refused, got {other:?}"),
        }
        match display.set_mode(&handle(), &mode()).unwrap_err() {
            MonitorError::Refused { capability, .. } => assert_eq!(capability, "modes"),
            other => panic!("expected Refused, got {other:?}"),
        }
        match display.set_layout(&placements()).unwrap_err() {
            MonitorError::Refused { capability, .. } => assert_eq!(capability, "layout"),
            other => panic!("expected Refused, got {other:?}"),
        }
    }

    #[test]
    fn io_failures_stay_display_errors() {
        let display = SharedDisplay::with_platform(FakePlatform::failing(Failure::Io));
        match display.snapshot().unwrap_err() {
            MonitorError::Display(DisplayError::Io(error)) => {
                assert_eq!(error.to_string(), "the bus closed");
            }
            other => panic!("expected a display Io error, got {other:?}"),
        }
    }
}
