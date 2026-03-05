use anyhow::{Context, Result};
use std::path::Path;
use std::time::Duration;

pub(super) async fn run_git<I, S>(
    args: I,
    current_dir: Option<&Path>,
    timeout: Duration,
    operation: &str,
) -> Result<std::process::Output>
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    let mut command = tokio::process::Command::new("git");
    command.args(args);
    if let Some(current_dir) = current_dir {
        command.current_dir(current_dir);
    }
    tokio::time::timeout(timeout, command.output())
        .await
        .with_context(|| format!("Git {} timed out", operation))?
        .with_context(|| format!("Failed to run git {}", operation))
}

pub(super) async fn run_git_checked<I, S>(
    args: I,
    current_dir: Option<&Path>,
    operation: &str,
) -> Result<std::process::Output>
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    let output = run_git(args, current_dir, super::GIT_TIMEOUT, operation).await?;
    if output.status.success() {
        return Ok(output);
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    anyhow::bail!("Git {} failed: {}", operation, stderr)
}

pub(super) async fn run_cargo_build(
    manifest_path: &Path,
    plugin_dir: &Path,
) -> Result<std::process::Output> {
    let jobs = cargo_build_jobs();
    let mut command = tokio::process::Command::new("cargo");
    command
        .arg("build")
        .arg("--release")
        .arg("--jobs")
        .arg(jobs.to_string())
        .arg("--manifest-path")
        .arg(manifest_path)
        .current_dir(plugin_dir);
    tokio::time::timeout(super::CARGO_BUILD_TIMEOUT, command.output())
        .await
        .context("Cargo build timed out")?
        .context("Failed to run cargo build")
}

fn cargo_build_jobs() -> usize {
    if let Ok(raw) = std::env::var("QOL_TRAY_CARGO_BUILD_JOBS") {
        if let Ok(parsed) = raw.parse::<usize>() {
            if parsed > 0 {
                return parsed;
            }
        }
    }

    std::thread::available_parallelism()
        .map(|n| n.get().min(super::DEFAULT_CARGO_BUILD_JOBS))
        .unwrap_or(super::DEFAULT_CARGO_BUILD_JOBS)
        .max(1)
}
