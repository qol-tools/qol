pub use qol_app_icon::RgbaImage;

#[derive(Debug, Clone)]
pub struct WindowInfo {
    pub id: u32,
    pub title: String,
    pub app_name: String,
    pub preview_path: Option<String>,
    #[allow(dead_code)] // read on Linux, not on macOS
    pub icon: Option<RgbaImage>,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub is_minimized: bool,
}

/// Strategy trait implemented once per OS. The picker calls `visible_windows`
/// on every show; no caching, no events, no polling. MRU order is whatever
/// the OS says is on top right now.
pub trait WindowDiscovery {
    /// Live, per-window, MRU-ordered snapshot. First entry = most recently
    /// active real window. Per-window means an app with three windows produces
    /// three entries — never grouped.
    ///
    /// Must return `Err` only when the OS is genuinely unsupported, not for
    /// transient failures (transient failures return an empty Vec).
    fn visible_windows(&self, include_minimized: bool) -> Result<Vec<WindowInfo>, DiscoveryError>;
}

#[derive(Debug)]
pub enum DiscoveryError {
    #[allow(dead_code)] // only constructed on Windows / unsupported hosts
    Unsupported,
}

impl std::fmt::Display for DiscoveryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DiscoveryError::Unsupported => {
                write!(f, "window discovery is not implemented on this platform")
            }
        }
    }
}

impl std::error::Error for DiscoveryError {}

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
pub(crate) mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
pub use linux::Platform;
#[cfg(target_os = "macos")]
pub use macos::Platform;
#[cfg(target_os = "windows")]
pub use windows::Platform;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
mod unsupported {
    use super::{DiscoveryError, WindowDiscovery, WindowInfo};
    pub struct Platform;
    impl WindowDiscovery for Platform {
        fn visible_windows(&self, _: bool) -> Result<Vec<WindowInfo>, DiscoveryError> {
            Err(DiscoveryError::Unsupported)
        }
    }
}
#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
pub use unsupported::Platform;
