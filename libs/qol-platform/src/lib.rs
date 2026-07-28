use std::path::PathBuf;

pub mod cinnamon;
mod platform;

use platform::PlatformApi;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinuxDisplayBackend {
    X11,
    Wayland,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlatformCapabilities {
    pub can_global_hotkey: bool,
    pub can_focus_popup: bool,
    pub can_clipboard_monitor: bool,
    pub can_window_positioning: bool,
}

pub fn linux_display_backend() -> LinuxDisplayBackend {
    platform::Platform.linux_display_backend()
}

pub fn current_capabilities() -> PlatformCapabilities {
    platform::Platform.current_capabilities()
}

pub fn launch_working_dir() -> Option<PathBuf> {
    platform::Platform.launch_working_dir()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiskSpace {
    pub available: u64,
    pub total: u64,
}

pub fn disk_space(path: &std::path::Path) -> std::io::Result<DiskSpace> {
    platform::Platform.disk_space(path)
}

#[cfg(test)]
mod disk_space_tests {
    use super::*;

    #[test]
    fn disk_space_reports_consistent_values() {
        let space = disk_space(&std::env::temp_dir()).expect("disk_space should succeed");
        assert!(space.total > 0, "total should be positive");
        assert!(
            space.available <= space.total,
            "available {} should not exceed total {}",
            space.available,
            space.total
        );
    }
}
