use super::ActionTransportPlatform;
use qol_runtime::local_ipc::LocalStream;
use std::path::Path;
use std::time::Duration;
use windows_sys::Win32::UI::WindowsAndMessaging::{AllowSetForegroundWindow, ASFW_ANY};

pub(super) struct Platform;

impl ActionTransportPlatform for Platform {
    fn default_io_timeout() -> Duration {
        Duration::from_secs(10)
    }

    /// AF_UNIX on Windows has no bounded connect; a path nobody listens on
    /// fails at once.
    fn connect(endpoint: &Path, _timeout: Duration) -> Result<LocalStream, ()> {
        LocalStream::connect(endpoint).map_err(|_| ())
    }

    /// The daemon takes foreground for the window it opens, and Windows only
    /// lets it while this process, which holds the user's last input, allows it.
    fn before_forward() {
        let _ = unsafe { AllowSetForegroundWindow(ASFW_ANY) };
    }
}
