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
}

pub fn spawn_piped(command: &mut Command) -> Result<CargoChild, std::io::Error> {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = command.spawn()?;
    let stdout = child.stdout.take().expect("stdout piped");
    let stderr = child.stderr.take().expect("stderr piped");
    Ok(CargoChild {
        child,
        stdout,
        stderr,
    })
}

pub const BUILD_TIMEOUT: Duration = Duration::from_secs(120);

pub fn wait_with_timeout(child: &mut Child, timeout: Duration) -> Result<bool, String> {
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status.success()),
            Ok(None) if start.elapsed() >= timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("Build timed out after {}s", timeout.as_secs()));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(250)),
            Err(e) => return Err(format!("Failed waiting for build: {}", e)),
        }
    }
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
    match wait_with_timeout(child, BUILD_TIMEOUT) {
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
}
