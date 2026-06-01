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

#[derive(Debug)]
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
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        exit_code: output.status.code().unwrap_or(-1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn action(command: &str, timeout: u64, cwd: Option<&str>) -> ActionConfig {
        ActionConfig {
            name: "t".to_string(),
            description: String::new(),
            command: command.to_string(),
            timeout,
            cwd: cwd.map(String::from),
        }
    }

    fn no_params() -> HashMap<String, String> {
        HashMap::new()
    }

    #[test]
    fn execution_spec_applies_interpolation_to_command_and_cwd() {
        let cfg = action("echo {{name}}", 42, Some("/tmp/{{dir}}"));
        let mut params = HashMap::new();
        params.insert("name".to_string(), "world".to_string());
        params.insert("dir".to_string(), "sub".to_string());
        let request = ExecutionRequest::new("a", &cfg, &params);

        let spec = execution_spec(&request);

        assert_eq!(spec.command, "echo world");
        assert_eq!(spec.cwd.as_deref(), Some("/tmp/sub"));
        assert_eq!(spec.timeout, 42);
    }

    #[test]
    fn execution_spec_leaves_cwd_none_when_unset() {
        let cfg = action("ls", 10, None);
        let params = no_params();
        let request = ExecutionRequest::new("a", &cfg, &params);
        let spec = execution_spec(&request);
        assert!(spec.cwd.is_none());
    }

    #[test]
    fn timeout_error_message_includes_timeout_in_seconds() {
        assert!(timeout_error(5).contains("5s"));
        assert!(timeout_error(0).contains("0s"));
    }

    #[test]
    fn command_error_prefixes_with_command_failed() {
        let err = std::io::Error::new(std::io::ErrorKind::NotFound, "not found");
        assert!(command_error(err).starts_with("Command failed:"));
    }

    #[cfg(unix)]
    mod unix_integration {
        use super::*;

        #[tokio::test]
        async fn execute_succeeds_for_trivial_command() {
            let cfg = action("true", 5, None);
            let result = execute(ExecutionRequest::new("t", &cfg, &no_params()))
                .await
                .unwrap();
            assert!(result.success);
            assert_eq!(result.exit_code, 0);
            assert!(result.stdout.is_empty());
            assert!(result.stderr.is_empty());
        }

        #[tokio::test]
        async fn execute_reports_failure_and_exit_code_for_nonzero_exit() {
            let cases = [("false", 1), ("exit 42", 42), ("exit 7", 7)];
            for (command, expected_exit) in cases {
                let cfg = action(command, 5, None);
                let result = execute(ExecutionRequest::new("t", &cfg, &no_params()))
                    .await
                    .unwrap();
                assert!(!result.success, "command={command}");
                assert_eq!(result.exit_code, expected_exit, "command={command}");
            }
        }

        #[tokio::test]
        async fn execute_captures_stdout_and_stderr_separately() {
            let cfg = action("echo out; echo err 1>&2", 5, None);
            let result = execute(ExecutionRequest::new("t", &cfg, &no_params()))
                .await
                .unwrap();
            assert_eq!(result.stdout.trim(), "out");
            assert_eq!(result.stderr.trim(), "err");
            assert!(result.success);
        }

        #[tokio::test]
        async fn execute_times_out_when_command_outlasts_timeout() {
            let cfg = action("sleep 5", 1, None);
            let err = execute(ExecutionRequest::new("t", &cfg, &no_params()))
                .await
                .unwrap_err();
            assert!(err.contains("Timeout"), "err: {err}");
            assert!(err.contains("1s"), "err: {err}");
        }

        #[tokio::test]
        async fn execute_runs_in_specified_cwd() {
            let tmp = tempfile::TempDir::new().unwrap();
            let cwd = tmp.path().display().to_string();
            let cfg = action("pwd", 5, Some(&cwd));
            let result = execute(ExecutionRequest::new("t", &cfg, &no_params()))
                .await
                .unwrap();
            let actual = std::path::PathBuf::from(result.stdout.trim())
                .canonicalize()
                .unwrap();
            let expected = tmp.path().canonicalize().unwrap();
            assert_eq!(actual, expected);
        }

        #[tokio::test]
        async fn execute_interpolates_command_params_into_shell_call() {
            let cfg = action("echo {{name}}", 5, None);
            let mut params = HashMap::new();
            params.insert("name".to_string(), "world".to_string());
            let result = execute(ExecutionRequest::new("t", &cfg, &params))
                .await
                .unwrap();
            assert_eq!(result.stdout.trim(), "world");
        }

        #[tokio::test]
        async fn execute_interpolates_cwd_params() {
            let tmp = tempfile::TempDir::new().unwrap();
            let cfg = action("pwd", 5, Some("{{dir}}"));
            let mut params = HashMap::new();
            params.insert("dir".to_string(), tmp.path().display().to_string());
            let result = execute(ExecutionRequest::new("t", &cfg, &params))
                .await
                .unwrap();
            let actual = std::path::PathBuf::from(result.stdout.trim())
                .canonicalize()
                .unwrap();
            let expected = tmp.path().canonicalize().unwrap();
            assert_eq!(actual, expected);
        }

        #[tokio::test]
        async fn execute_returns_io_error_when_cwd_does_not_exist() {
            let cfg = action("true", 5, Some("/this/path/does/not/exist/anywhere"));
            let err = execute(ExecutionRequest::new("t", &cfg, &no_params()))
                .await
                .unwrap_err();
            assert!(err.contains("Command failed"), "err: {err}");
        }

        #[tokio::test]
        async fn execute_shell_escapes_param_values_to_prevent_injection() {
            let cfg = action("echo {{value}}", 5, None);
            let mut params = HashMap::new();
            params.insert(
                "value".to_string(),
                "; rm -rf /tmp/qol-tray-injection-canary".to_string(),
            );
            let result = execute(ExecutionRequest::new("t", &cfg, &params))
                .await
                .unwrap();
            assert!(
                result.stdout.contains("; rm -rf"),
                "shell metacharacters must arrive at echo as one literal arg, stdout: {:?}",
                result.stdout,
            );
        }
    }
}
