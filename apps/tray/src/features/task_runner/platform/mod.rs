use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
mod fallback;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
mod macos_command_tree;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod unix;
#[cfg(not(target_os = "macos"))]
mod verified_command_tree;
#[cfg(target_os = "windows")]
mod windows;
#[cfg(any(target_os = "windows", test))]
mod windows_quote;

#[cfg(target_os = "macos")]
pub(super) use macos_command_tree::CommandTree;
#[cfg(not(target_os = "macos"))]
pub(super) use verified_command_tree::CommandTree;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
pub(super) use fallback::Platform;
#[cfg(target_os = "linux")]
pub(super) use linux::Platform;
#[cfg(target_os = "macos")]
pub(super) use macos::Platform;
#[cfg(target_os = "windows")]
pub(super) use windows::Platform;

trait TaskRunnerPlatform {
    fn shell_command(&self, script: &str) -> Result<Command, String>;
    fn render_command(
        &self,
        template: &str,
        params: &HashMap<String, String>,
    ) -> Result<String, String>;
    fn guardian_executable(&self) -> std::io::Result<PathBuf>;
}

pub(super) fn shell_command(script: &str) -> Result<Command, String> {
    Platform.shell_command(script)
}

pub(super) fn render_command(
    template: &str,
    params: &HashMap<String, String>,
) -> Result<String, String> {
    Platform.render_command(template, params)
}

pub(super) fn verified_process_tree() -> std::io::Result<qol_process::ProcessTreeGuard> {
    let guardian = guardian_command(&Platform.guardian_executable()?);
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
