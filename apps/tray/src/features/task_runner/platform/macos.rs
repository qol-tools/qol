use std::path::PathBuf;
use std::process::Command;

use super::TaskRunnerPlatform;

pub(in crate::features::task_runner) struct Platform;

impl TaskRunnerPlatform for Platform {
    fn shell_command(&self, script: &str) -> Result<Command, String> {
        let mut command = Command::new("sh");
        command.arg("-c").arg(script);
        Ok(command)
    }

    fn quote_param(&self, value: &str) -> Result<String, String> {
        Ok(super::unix::quote(value))
    }

    fn guardian_executable(&self) -> std::io::Result<PathBuf> {
        std::env::current_exe()
    }
}
