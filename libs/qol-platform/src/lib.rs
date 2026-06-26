use std::path::PathBuf;

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

#[cfg(target_os = "linux")]
pub fn linux_display_backend() -> LinuxDisplayBackend {
    let session = std::env::var("XDG_SESSION_TYPE")
        .unwrap_or_default()
        .to_ascii_lowercase();
    let has_wayland = std::env::var_os("WAYLAND_DISPLAY").is_some() || session == "wayland";
    if has_wayland {
        return LinuxDisplayBackend::Wayland;
    }
    let has_x11 = std::env::var_os("DISPLAY").is_some() || session == "x11";
    if has_x11 {
        return LinuxDisplayBackend::X11;
    }
    LinuxDisplayBackend::Unknown
}

#[cfg(not(target_os = "linux"))]
pub fn linux_display_backend() -> LinuxDisplayBackend {
    LinuxDisplayBackend::Unknown
}

pub fn current_capabilities() -> PlatformCapabilities {
    #[cfg(target_os = "linux")]
    {
        match linux_display_backend() {
            LinuxDisplayBackend::X11 => PlatformCapabilities {
                can_global_hotkey: true,
                can_focus_popup: true,
                can_clipboard_monitor: true,
                can_window_positioning: true,
            },
            LinuxDisplayBackend::Wayland => PlatformCapabilities {
                can_global_hotkey: false,
                can_focus_popup: true,
                can_clipboard_monitor: false,
                can_window_positioning: false,
            },
            LinuxDisplayBackend::Unknown => PlatformCapabilities {
                can_global_hotkey: false,
                can_focus_popup: false,
                can_clipboard_monitor: false,
                can_window_positioning: false,
            },
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        PlatformCapabilities {
            can_global_hotkey: true,
            can_focus_popup: true,
            can_clipboard_monitor: true,
            can_window_positioning: true,
        }
    }
}

pub fn launch_working_dir() -> Option<PathBuf> {
    dirs::home_dir().or_else(|| std::env::current_dir().ok())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiskSpace {
    pub available: u64,
    pub total: u64,
}

#[cfg(target_os = "macos")]
pub fn disk_space(path: &std::path::Path) -> std::io::Result<DiskSpace> {
    let c_path = unix_c_path(path)?;
    let mut stat: libc::statfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statfs(c_path.as_ptr(), &mut stat) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    let block = stat.f_bsize as u64;
    Ok(DiskSpace {
        available: stat.f_bavail * block,
        total: stat.f_blocks * block,
    })
}

#[cfg(all(unix, not(target_os = "macos")))]
pub fn disk_space(path: &std::path::Path) -> std::io::Result<DiskSpace> {
    let c_path = unix_c_path(path)?;
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c_path.as_ptr(), &mut stat) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    let block = stat.f_frsize as u64;
    Ok(DiskSpace {
        available: stat.f_bavail as u64 * block,
        total: stat.f_blocks as u64 * block,
    })
}

#[cfg(unix)]
fn unix_c_path(path: &std::path::Path) -> std::io::Result<std::ffi::CString> {
    use std::os::unix::ffi::OsStrExt;
    std::ffi::CString::new(path.as_os_str().as_bytes()).map_err(|_| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "path contains NUL byte")
    })
}

#[cfg(windows)]
pub fn disk_space(path: &std::path::Path) -> std::io::Result<DiskSpace> {
    use std::os::windows::ffi::OsStrExt;

    #[link(name = "kernel32")]
    extern "system" {
        fn GetDiskFreeSpaceExW(
            directory_name: *const u16,
            free_bytes_available_to_caller: *mut u64,
            total_number_of_bytes: *mut u64,
            total_number_of_free_bytes: *mut u64,
        ) -> i32;
    }

    let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
    wide.push(0);
    let mut available: u64 = 0;
    let mut total: u64 = 0;
    let ok = unsafe {
        GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &mut available,
            &mut total,
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(DiskSpace { available, total })
}

#[cfg(not(any(unix, windows)))]
pub fn disk_space(_path: &std::path::Path) -> std::io::Result<DiskSpace> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "disk_space is not supported on this platform",
    ))
}

#[cfg(test)]
mod disk_space_tests {
    use super::*;

    #[test]
    fn disk_space_reports_plausible_values() {
        let space = disk_space(&std::env::temp_dir()).expect("disk_space should succeed");
        assert!(space.total > 0, "total should be positive");
        assert!(space.available > 0, "available should be positive");
        assert!(
            space.available <= space.total,
            "available {} should not exceed total {}",
            space.available,
            space.total
        );
        eprintln!(
            "disk_space: available={} GB total={} GB",
            space.available / 1_000_000_000,
            space.total / 1_000_000_000
        );
    }
}
