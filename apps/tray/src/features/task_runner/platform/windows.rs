use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;

use super::TaskRunnerPlatform;

pub(in crate::features::task_runner) struct Platform;

impl TaskRunnerPlatform for Platform {
    fn shell_command(&self, script: &str) -> Result<Command, String> {
        let mut command = Command::new("cmd");
        command.args(["/S", "/C"]).raw_arg(format!("\"{script}\""));
        Ok(command)
    }

    fn quote_param(&self, value: &str) -> Result<String, String> {
        super::windows_quote::quote(value)
    }

    fn guardian_executable(&self) -> std::io::Result<PathBuf> {
        std::env::current_exe()
    }
}
