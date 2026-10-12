use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;

use super::super::interpolation::interpolate_shell;
use super::TaskRunnerPlatform;

pub(in crate::features::task_runner) struct Platform;

impl TaskRunnerPlatform for Platform {
    fn shell_command(&self, script: &str) -> Result<Command, String> {
        let mut command = Command::new("sh");
        command.arg("-c").arg(script);
        Ok(command)
    }

    fn render_command(
        &self,
        template: &str,
        params: &HashMap<String, String>,
    ) -> Result<String, String> {
        interpolate_shell(template, params, |value| Ok(super::unix::quote(value)))
    }

    fn guardian_executable(&self) -> std::io::Result<PathBuf> {
        std::env::current_exe()
    }
}
