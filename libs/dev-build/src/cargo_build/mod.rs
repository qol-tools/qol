mod codesign;
mod messages;
mod plugin_build;

use std::path::Path;
use std::process::{Child, ChildStderr, ChildStdout, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use crate::adapters::CargoPluginBuilder;
use crate::types::BuildResult;

pub use messages::{
    parse_cargo_message, select_binary_executable, CargoArtifact, CargoArtifactSelectionError,
    CargoMessage, CargoMessageError,
};

pub struct CargoBuildOutput {
    pub artifacts: Vec<CargoArtifact>,
    pub diagnostics: String,
}

#[derive(Debug)]
pub enum CargoBuildCommandError {
    Spawn(std::io::Error),
    InvalidUtf8(std::string::FromUtf8Error),
    InvalidMessage {
        line: usize,
        source: CargoMessageError,
    },
    Failed {
        status: ExitStatus,
        output: String,
    },
}

impl std::fmt::Display for CargoBuildCommandError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Spawn(error) => write!(formatter, "failed to run Cargo: {error}"),
            Self::InvalidUtf8(error) => write!(formatter, "Cargo returned invalid UTF-8: {error}"),
            Self::InvalidMessage { line, source } => {
                write!(formatter, "Cargo message {line} is invalid: {source}")
            }
            Self::Failed { status, output } => {
                write!(formatter, "Cargo failed with {status}: {}", output.trim())
            }
        }
    }
}

impl std::error::Error for CargoBuildCommandError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Spawn(error) => Some(error),
            Self::InvalidUtf8(error) => Some(error),
            Self::InvalidMessage { source, .. } => Some(source),
            Self::Failed { .. } => None,
        }
    }
}

pub fn run_cargo_command(
    command: &mut Command,
) -> Result<CargoBuildOutput, CargoBuildCommandError> {
    if !command
        .get_args()
        .any(|arg| arg.to_string_lossy().starts_with("--message-format"))
    {
        command
            .arg("--message-format")
            .arg("json-render-diagnostics");
    }
    let output = command.output().map_err(CargoBuildCommandError::Spawn)?;
    let stdout = String::from_utf8(output.stdout).map_err(CargoBuildCommandError::InvalidUtf8)?;
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let mut artifacts = Vec::new();
    let mut diagnostics = String::new();
    for (index, line) in stdout.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        match parse_cargo_message(line).map_err(|source| {
            CargoBuildCommandError::InvalidMessage {
                line: index + 1,
                source,
            }
        })? {
            CargoMessage::Artifact(artifact) => artifacts.push(artifact),
            CargoMessage::Diagnostic(rendered) => diagnostics.push_str(&rendered),
            CargoMessage::Other => {}
        }
    }
    diagnostics.push_str(&stderr);
    if !output.status.success() {
        return Err(CargoBuildCommandError::Failed {
            status: output.status,
            output: diagnostics,
        });
    }
    Ok(CargoBuildOutput {
        artifacts,
        diagnostics,
    })
}

pub struct CargoChild {
    pub child: Child,
    pub stdout: ChildStdout,
    pub stderr: ChildStderr,
    pub(crate) process_tree: qol_process::OwnedProcessTree,
}

pub fn spawn_piped(mut command: Command) -> Result<CargoChild, std::io::Error> {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let (mut child, process_tree) = qol_process::spawn_owned(command)?;
    let stdout = child.stdout.take().expect("stdout piped");
    let stderr = child.stderr.take().expect("stderr piped");
    Ok(CargoChild {
        child,
        stdout,
        stderr,
        process_tree,
    })
}

pub const BUILD_TIMEOUT: Duration = Duration::from_secs(120);
pub const BUILD_TERMINATION_GRACE: Duration = Duration::from_secs(2);

const BUILD_WAIT_INTERVAL: Duration = Duration::from_millis(50);

pub fn wait_with_timeout(child: &mut Child, timeout: Duration) -> Result<bool, String> {
    wait_with_timeout_and_poll("Cargo", child, timeout, || {})
}

pub fn wait_with_timeout_and_poll<F>(
    plugin_id: &str,
    child: &mut Child,
    timeout: Duration,
    on_poll: F,
) -> Result<bool, String>
where
    F: FnMut(),
{
    wait_with_timeout_and_poll_inner(plugin_id, child, None, timeout, on_poll)
}

pub fn wait_with_timeout_and_poll_owned<F>(
    plugin_id: &str,
    child: &mut Child,
    process_tree: &qol_process::OwnedProcessTree,
    timeout: Duration,
    on_poll: F,
) -> Result<bool, String>
where
    F: FnMut(),
{
    wait_with_timeout_and_poll_inner(plugin_id, child, Some(process_tree), timeout, on_poll)
}

fn wait_with_timeout_and_poll_inner<F>(
    plugin_id: &str,
    child: &mut Child,
    process_tree: Option<&qol_process::OwnedProcessTree>,
    timeout: Duration,
    mut on_poll: F,
) -> Result<bool, String>
where
    F: FnMut(),
{
    let start = Instant::now();
    loop {
        on_poll();
        match child.try_wait() {
            Ok(Some(status)) => {
                let tree_exited = match tree_has_exited(child, process_tree) {
                    Ok(exited) => exited,
                    Err(error) => {
                        let message = format!("Failed checking Cargo process tree: {error}");
                        return match terminate_owned_tree(
                            plugin_id,
                            child,
                            process_tree,
                            "process-tree check error",
                        ) {
                            Ok(()) => Err(message),
                            Err(cleanup_error) => Err(format!("{message}; {cleanup_error}")),
                        };
                    }
                };
                if !tree_exited {
                    log::warn!(
                        "[dev-build] event=cancellation plugin_id={} reason=orphaned_process_tree elapsed_ms={}",
                        plugin_id,
                        start.elapsed().as_millis()
                    );
                    terminate_owned_tree(plugin_id, child, process_tree, "orphaned process tree")?;
                }
                return Ok(status.success());
            }
            Ok(None) if start.elapsed() >= timeout => {
                let message = format!("Build timed out after {:?}", timeout);
                log::warn!(
                    "[dev-build] event=cancellation plugin_id={} reason=timeout elapsed_ms={} action=terminate_process_tree",
                    plugin_id,
                    start.elapsed().as_millis()
                );
                return match terminate_owned_tree(plugin_id, child, process_tree, "timeout") {
                    Ok(()) => Err(message),
                    Err(error) => Err(format!("{message}; {error}")),
                };
            }
            Ok(None) => std::thread::sleep(BUILD_WAIT_INTERVAL),
            Err(error) => {
                let message = format!("Failed waiting for build: {error}");
                return match terminate_owned_tree(plugin_id, child, process_tree, "wait error") {
                    Ok(()) => Err(message),
                    Err(cleanup_error) => Err(format!("{message}; {cleanup_error}")),
                };
            }
        }
    }
}

fn tree_has_exited(
    child: &Child,
    process_tree: Option<&qol_process::OwnedProcessTree>,
) -> Result<bool, String> {
    match process_tree {
        Some(process_tree) => process_tree
            .tree_has_exited()
            .map_err(|error| format!("failed to inspect owned process tree: {error}")),
        None => Ok(!qol_process::is_group_alive(child.id())),
    }
}

fn terminate_owned_tree(
    plugin_id: &str,
    child: &mut Child,
    process_tree: Option<&qol_process::OwnedProcessTree>,
    reason: &str,
) -> Result<(), String> {
    let pid = child.id();
    let termination = match process_tree {
        Some(process_tree) => process_tree.terminate_and_wait(child, BUILD_TERMINATION_GRACE),
        None => {
            let termination = qol_process::terminate_owned(child, BUILD_TERMINATION_GRACE);
            if qol_process::is_group_alive(pid) {
                qol_process::terminate_group(pid, BUILD_TERMINATION_GRACE);
            }
            termination
        }
    };
    let process_alive = qol_process::is_pid_alive(pid);
    let tree_alive = tree_has_exited(child, process_tree)
        .map(|exited| !exited)
        .unwrap_or(true);
    if let Err(error) = termination {
        log::error!(
            "[dev-build] event=reap plugin_id={} reason={} pid={} reaped=false process_tree_alive={} error={}",
            plugin_id,
            reason,
            pid,
            process_alive || tree_alive,
            error
        );
        return Err(format!(
            "failed to terminate and reap Cargo process tree (pid {pid}): {error}"
        ));
    }
    if process_alive || tree_alive {
        log::error!(
            "[dev-build] event=reap plugin_id={} reason={} pid={} reaped=false process_tree_alive=true",
            plugin_id,
            reason,
            pid
        );
        return Err(format!(
            "Cargo process tree is still alive after termination (pid {pid})"
        ));
    }
    log::warn!(
        "[dev-build] event=reap plugin_id={} reason={} pid={} reaped=true process_tree_alive=false",
        plugin_id,
        reason,
        pid
    );
    Ok(())
}

pub fn finish_build<F>(
    plugin_id: &str,
    child: &mut Child,
    output: String,
    on_progress: &mut F,
    on_success: impl FnOnce(String, &mut F) -> BuildResult,
) -> BuildResult
where
    F: FnMut(u8, String),
{
    let wait_result = wait_with_timeout_and_poll(plugin_id, child, BUILD_TIMEOUT, || {});
    finish_build_after_wait(plugin_id, wait_result, output, on_progress, on_success)
}

pub fn finish_build_owned<F>(
    plugin_id: &str,
    child: &mut Child,
    process_tree: &qol_process::OwnedProcessTree,
    output: String,
    on_progress: &mut F,
    on_success: impl FnOnce(String, &mut F) -> BuildResult,
) -> BuildResult
where
    F: FnMut(u8, String),
{
    let wait_result =
        wait_with_timeout_and_poll_owned(plugin_id, child, process_tree, BUILD_TIMEOUT, || {});
    finish_build_after_wait(plugin_id, wait_result, output, on_progress, on_success)
}

pub(crate) fn finish_build_after_wait<F>(
    plugin_id: &str,
    wait_result: Result<bool, String>,
    output: String,
    on_progress: &mut F,
    on_success: impl FnOnce(String, &mut F) -> BuildResult,
) -> BuildResult
where
    F: FnMut(u8, String),
{
    match wait_result {
        Ok(true) => on_success(output, on_progress),
        Ok(false) => failed_status_build(plugin_id, output),
        Err(message) => failed_build(plugin_id, message),
    }
}

pub fn failed_build(plugin_id: &str, output: String) -> BuildResult {
    build_result(plugin_id, false, output)
}

pub fn failed_status_build(plugin_id: &str, output: String) -> BuildResult {
    log::error!("Cargo build failed for {}:\n{}", plugin_id, output);
    build_result(plugin_id, false, output)
}

pub fn finished_build(plugin_id: &str, output: String) -> BuildResult {
    build_result(plugin_id, true, output)
}

fn build_result(plugin_id: &str, success: bool, output: String) -> BuildResult {
    BuildResult {
        plugin_id: plugin_id.to_string(),
        success,
        output,
        skipped: false,
        artifacts: Vec::new(),
    }
}

pub struct CargoCommandPluginBuilder;

impl CargoPluginBuilder for CargoCommandPluginBuilder {
    fn build_plugin_with_progress(
        &self,
        plugin_id: &str,
        path: &Path,
        on_progress: &mut dyn FnMut(u8, String),
    ) -> BuildResult {
        plugin_build::build_cargo_plugin_with_progress(plugin_id, path, on_progress)
    }

    fn build_plugins_with_progress(
        &self,
        plugins: &[(&str, &Path)],
        on_progress: &mut dyn FnMut(&str, u8, String),
    ) -> Option<Vec<BuildResult>> {
        Some(plugin_build::build_cargo_plugins_with_progress(
            plugins,
            on_progress,
        ))
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::process::Command;

    struct ChildGuard(Option<Child>);

    impl Drop for ChildGuard {
        fn drop(&mut self) {
            if let Some(mut child) = self.0.take() {
                let _ = qol_process::terminate_owned(&mut child, Duration::from_millis(20));
            }
        }
    }

    #[test]
    fn timeout_terminates_and_reaps_the_owned_process_tree() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let child_pid_path = temp.path().join("child.pid");
        let mut command = Command::new("sh");
        command
            .args([
                "-c",
                "sleep 30 & child=$!; printf '%s' \"$child\" > \"$QOL_TEST_CHILD_PID\"; wait",
            ])
            .env("QOL_TEST_CHILD_PID", &child_pid_path);
        let CargoChild {
            child,
            stdout,
            stderr,
            process_tree,
        } = spawn_piped(command).expect("spawn owned process");
        drop(stdout);
        drop(stderr);
        let mut guard = ChildGuard(Some(child));
        let root_pid = guard.0.as_ref().expect("child guard").id();
        let deadline = Instant::now() + Duration::from_secs(2);
        while !child_pid_path.is_file() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(child_pid_path.is_file(), "child pid was not recorded");
        let child_pid = std::fs::read_to_string(&child_pid_path)
            .expect("child pid file")
            .trim()
            .parse::<u32>()
            .expect("child pid");

        let result = wait_with_timeout_and_poll_owned(
            "timeout-test",
            guard.0.as_mut().expect("child guard"),
            &process_tree,
            Duration::from_millis(100),
            || {},
        );
        assert!(result.is_err(), "the hanging process must time out");
        assert!(!qol_process::is_pid_alive(root_pid));
        assert!(!qol_process::is_group_alive(root_pid));
        let deadline = Instant::now() + Duration::from_secs(2);
        while qol_process::is_pid_alive(child_pid) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            !qol_process::is_pid_alive(child_pid),
            "the descendant must not survive Cargo cancellation"
        );
    }
}
