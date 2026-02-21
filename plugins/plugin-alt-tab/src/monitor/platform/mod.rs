use gpui::*;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;

pub(crate) trait PlatformQueries: Send + Sync {
    fn cursor_position(&self) -> Option<(f32, f32)>;
    fn focused_window_bounds(&self) -> Option<Bounds<Pixels>>;
    fn physical_monitors(&self) -> Vec<Bounds<Pixels>>;

    /// Whether `focused_window_bounds()` is safe to call from a persistent
    /// background polling thread. Returns `false` on platforms where the
    /// underlying API can deadlock with the UI toolkit's render loop.
    fn poll_focused_window(&self) -> bool {
        true
    }
}

#[cfg(target_os = "linux")]
pub(crate) fn create() -> impl PlatformQueries {
    linux::LinuxQueries::new()
}

#[cfg(target_os = "macos")]
pub(crate) fn create() -> impl PlatformQueries {
    macos::MacQueries::new(std::process::id() as i32)
}
