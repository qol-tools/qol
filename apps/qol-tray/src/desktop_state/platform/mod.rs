use qol_runtime::MonitorBounds;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;

pub(crate) trait Platform: Send + Sync {
    fn cursor_position(&self) -> Option<(f32, f32)>;
    fn focused_window_bounds(&self) -> Option<MonitorBounds>;
    fn physical_monitors(&self) -> Vec<MonitorBounds>;

    fn poll_focused_window(&self) -> bool {
        true
    }
}

#[cfg(target_os = "linux")]
pub(crate) fn create() -> impl Platform {
    linux::LinuxQueries::new()
}

#[cfg(target_os = "macos")]
pub(crate) fn create() -> impl Platform {
    macos::MacQueries::new(std::process::id() as i32)
}
