use std::collections::HashMap;
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;

use super::TaskRunnerPlatform;

pub(in crate::features::task_runner) struct Platform;

impl TaskRunnerPlatform for Platform {
    fn shell_command(&self, script: &str) -> Result<Command, String> {
        let mut command = qol_process::system_command("cmd.exe");
        command
            .args(["/D", "/V:OFF", "/S", "/C"])
            .raw_arg(format!("\"{script}\""));
        Ok(command)
    }

    fn render_command(
        &self,
        template: &str,
        params: &HashMap<String, String>,
    ) -> Result<String, String> {
        super::windows_quote::render(template, params)
    }

    fn guardian_executable(&self) -> std::io::Result<PathBuf> {
        std::env::current_exe()
    }
}
