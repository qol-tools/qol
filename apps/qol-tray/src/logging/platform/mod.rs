#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

use std::path::PathBuf;

pub fn log_dir() -> PathBuf {
    #[cfg(target_os = "linux")]
    return linux::log_dir();
    #[cfg(target_os = "macos")]
    return macos::log_dir();
    #[cfg(target_os = "windows")]
    return windows::log_dir();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_dir_is_absolute_and_ends_with_logs() {
        let dir = log_dir();
        assert!(dir.is_absolute(), "log dir {:?} should be absolute", dir);
        assert!(
            dir.ends_with("logs"),
            "log dir {:?} should end with 'logs'",
            dir
        );
    }

    #[test]
    fn log_dir_contains_app_name() {
        let dir = log_dir();
        let path_str = dir.to_string_lossy();
        assert!(
            path_str.contains("qol-tray"),
            "log dir {:?} should contain qol-tray",
            dir
        );
    }
}
