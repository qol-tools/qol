use gpui::*;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;

pub(crate) trait PlatformQueries: Send + Sync {
    fn cursor_position(&self) -> Option<(f32, f32)>;
    fn focused_window_bounds(&self) -> Option<Bounds<Pixels>>;
    fn physical_monitors(&self) -> Vec<Bounds<Pixels>>;
}

#[cfg(target_os = "linux")]
pub(crate) fn create() -> impl PlatformQueries {
    linux::LinuxQueries::new()
}

#[cfg(target_os = "macos")]
pub(crate) fn create() -> impl PlatformQueries {
    macos::MacQueries::new(std::process::id() as i32)
}
