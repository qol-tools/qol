use tokio::process::Command;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
compile_error!(
    "features::task_runner::platform::shell_command is not implemented for this target OS"
);

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub fn shell_command(script: &str) -> Command {
    let mut cmd = Command::new("sh");
    cmd.arg("-c").arg(script);
    cmd
}

#[cfg(target_os = "windows")]
pub fn shell_command(script: &str) -> Command {
    let mut cmd = Command::new("cmd");
    cmd.arg("/C").arg(script);
    cmd
}
