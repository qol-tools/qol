use std::path::PathBuf;
use std::process::Command;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
compile_error!(
    "features::task_runner::platform::shell_command is not implemented for this target OS"
);

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(super) fn shell_command(script: &str) -> Command {
    let mut command = Command::new("sh");
    command.arg("-c").arg(script);
    command
}

#[cfg(target_os = "windows")]
pub(super) fn shell_command(script: &str) -> Command {
    let mut command = Command::new("cmd");
    command.arg("/C").arg(script);
    command
}

pub(super) fn verified_process_tree() -> std::io::Result<qol_process::ProcessTreeGuard> {
    let guardian = guardian_command(&guardian_executable()?);
    qol_process::own_current_process_tree_with_guardian(guardian)
}

#[cfg(not(test))]
fn guardian_command(executable: &std::path::Path) -> Command {
    qol_process::process_tree_guardian_command(executable)
}

#[cfg(test)]
fn guardian_command(executable: &std::path::Path) -> Command {
    let mut command = Command::new(executable);
    command.args([
        "--exact",
        "features::task_runner::execution::command_task::tests::process_guardian_test_entry",
        "--nocapture",
    ]);
    command
}

#[cfg(target_os = "linux")]
fn guardian_executable() -> std::io::Result<PathBuf> {
    Ok(PathBuf::from("/proc/self/exe"))
}

#[cfg(not(target_os = "linux"))]
fn guardian_executable() -> std::io::Result<PathBuf> {
    std::env::current_exe()
}
