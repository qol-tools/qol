use std::path::PathBuf;

pub fn running_exe_path(pid: i32) -> Option<PathBuf> {
    super::platform::pid_exe_path(pid)
}
