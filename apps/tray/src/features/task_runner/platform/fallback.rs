use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;

use super::super::interpolation::interpolate_shell;
use super::TaskRunnerPlatform;

pub(in crate::features::task_runner) struct Platform;

const UNAVAILABLE: &str = "task-runner shell execution is unavailable on this platform";

impl TaskRunnerPlatform for Platform {
    fn shell_command(&self, _script: &str) -> Result<Command, String> {
        Err(UNAVAILABLE.to_string())
    }

    fn render_command(
        &self,
        template: &str,
        params: &HashMap<String, String>,
    ) -> Result<String, String> {
        interpolate_shell(template, params, |_| Err(UNAVAILABLE.to_string()))
    }

    fn guardian_executable(&self) -> std::io::Result<PathBuf> {
        std::env::current_exe()
    }
}
