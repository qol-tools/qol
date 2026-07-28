use std::path::{Path, PathBuf};
use std::process::Command;

pub(super) fn process_holds_handoff_resources(pid: u32) -> bool {
    if !crate::process_utils::is_pid_alive(pid as i32) || process_is_zombie(pid) {
        return false;
    }
    let Some(executable) = crate::plugins::daemon_tracker::running_exe_path(pid as i32) else {
        return false;
    };
    current_build_dir()
        .as_ref()
        .is_some_and(|dir| executable.starts_with(dir))
        || crate::plugins::daemon_tracker::ManagedRoots::load().contains(&executable)
}

fn current_build_dir() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
}

fn process_is_zombie(pid: u32) -> bool {
    let pid_arg = pid.to_string();
    let output = Command::new("ps")
        .args(["-p", pid_arg.as_str(), "-o", "stat="])
        .output();
    let Ok(output) = output else {
        return false;
    };
    if !output.status.success() {
        return false;
    }
    String::from_utf8_lossy(&output.stdout)
        .trim_start()
        .starts_with('Z')
}
