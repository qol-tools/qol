use super::config::ActionConfig;
use super::interpolation::{interpolate, interpolate_shell};
use super::platform;
use std::collections::HashMap;
use std::process::{Output, Stdio};
use tokio::process::Command;

pub(super) struct ExecutionRequest<'a> {
    action_id: &'a str,
    action: &'a ActionConfig,
    params: &'a HashMap<String, String>,
}

impl<'a> ExecutionRequest<'a> {
    pub(super) fn new(
        action_id: &'a str,
        action: &'a ActionConfig,
        params: &'a HashMap<String, String>,
    ) -> Self {
        Self {
            action_id,
            action,
            params,
        }
    }
}

pub(super) struct ExecutionResult {
    pub(super) success: bool,
    pub(super) stdout: String,
    pub(super) stderr: String,
    pub(super) exit_code: i32,
}

struct ExecutionSpec {
    command: String,
    cwd: Option<String>,
    timeout: u64,
}

pub(super) async fn execute(request: ExecutionRequest<'_>) -> Result<ExecutionResult, String> {
    let spec = execution_spec(&request);
    log::info!("[task-runner] {}: {}", request.action_id, spec.command);
    let output = command_output(&spec).await?;
    Ok(execution_result(output))
}

fn execution_spec(request: &ExecutionRequest<'_>) -> ExecutionSpec {
    ExecutionSpec {
        command: interpolate_shell(&request.action.command, request.params),
        cwd: request
            .action
            .cwd
            .as_ref()
            .map(|cwd| interpolate(cwd, request.params)),
        timeout: request.action.timeout,
    }
}

async fn command_output(spec: &ExecutionSpec) -> Result<Output, String> {
    let mut command = command(spec);
    run_command(&mut command, spec.timeout).await
}

fn command(spec: &ExecutionSpec) -> Command {
    let mut command = platform::shell_command(&spec.command);
    configure_io(&mut command);
    configure_cwd(&mut command, spec.cwd.as_deref());
    command
}

fn configure_io(command: &mut Command) {
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
}

fn configure_cwd(command: &mut Command, cwd: Option<&str>) {
    let Some(dir) = cwd else {
        return;
    };
    command.current_dir(dir);
}

async fn run_command(command: &mut Command, timeout: u64) -> Result<Output, String> {
    let duration = std::time::Duration::from_secs(timeout);
    let result = tokio::time::timeout(duration, command.output())
        .await
        .map_err(|_| timeout_error(timeout))?;
    result.map_err(command_error)
}

fn command_error(error: std::io::Error) -> String {
    let message = format!("Command failed: {error}");
    log::error!("[task-runner] {message}");
    message
}

fn timeout_error(timeout: u64) -> String {
    let message = format!("Timeout after {timeout}s");
    log::error!("[task-runner] Command timed out after {}s", timeout);
    message
}

fn execution_result(output: Output) -> ExecutionResult {
    ExecutionResult {
        success: output.status.success(),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        exit_code: output.status.code().unwrap_or(-1),
    }
}
