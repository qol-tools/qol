use std::path::PathBuf;
use std::process::Command;

mod process_tree;

pub(super) use process_tree::CommandTree;

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(super) fn shell_command(script: &str) -> Result<Command, String> {
    let mut command = Command::new("sh");
    command.arg("-c").arg(script);
    Ok(command)
}

#[cfg(target_os = "windows")]
pub(super) fn shell_command(script: &str) -> Result<Command, String> {
    let mut command = Command::new("cmd");
    command.arg("/C").arg(script);
    Ok(command)
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
pub(super) fn shell_command(_script: &str) -> Result<Command, String> {
    Err("task-runner shell execution is unavailable on this platform".to_string())
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
