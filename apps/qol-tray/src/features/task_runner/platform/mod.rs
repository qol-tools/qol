use tokio::process::Command;

#[cfg(any(target_os = "linux", target_os = "macos"))]
mod unix;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(any(target_os = "linux", target_os = "macos"))]
use unix as imp;
#[cfg(target_os = "windows")]
use windows as imp;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
compile_error!("task runner platform command adapter is required for this target OS");

pub fn shell_command(script: &str) -> Command {
    imp::shell_command(script)
}
