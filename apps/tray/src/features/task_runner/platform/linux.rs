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

    fn guardian_executable(&self) -> std::io::Result<PathBuf> {
        Ok(PathBuf::from("/proc/self/exe"))
    }
}
