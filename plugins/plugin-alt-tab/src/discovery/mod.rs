pub use qol_app_icon::RgbaImage;

#[derive(Debug, Clone)]
pub struct WindowInfo {
    pub id: u32,
    pub title: String,
    pub app_name: String,
    pub preview_path: Option<String>,
    pub icon: Option<RgbaImage>,
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
pub struct DiscoveryError;

impl std::fmt::Display for DiscoveryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "window discovery is not implemented on this platform")
    }
}

impl std::error::Error for DiscoveryError {}

pub(crate) mod platform;

pub use platform::Platform;
