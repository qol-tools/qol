use super::sanitize_git_environment;
use anyhow::{bail, Context, Result};
use std::path::Path;
use std::process::{Command, Output};

pub(super) fn git_stdout<const N: usize>(
    root: &Path,
    args: [&str; N],
    action: &str,
) -> Result<String> {
    let value = git_stdout_allow_empty(root, args, action)?;
    if value.is_empty() {
        bail!("git returned an empty {action}");
    }
    Ok(value)
}

pub(super) fn git_stdout_allow_empty<const N: usize>(
    root: &Path,
    args: [&str; N],
    action: &str,
) -> Result<String> {
    let mut command = Command::new("git");
    command.current_dir(root).args(args);
    sanitize_git_environment(&mut command);
    output_text(command_output(&mut command, action)?, action)
}

pub(super) fn command_success(command: &mut Command, action: &str) -> Result<()> {
    command_output(command, action).map(|_| ())
}

pub(super) fn command_output(command: &mut Command, action: &str) -> Result<Output> {
    let output = command
        .output()
        .with_context(|| format!("{action}: failed to spawn git"))?;
    if output.status.success() {
        return Ok(output);
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    bail!("{action} failed with {}: {}", output.status, stderr.trim())
}

pub(super) fn output_text(output: Output, label: &str) -> Result<String> {
    let value = String::from_utf8(output.stdout).with_context(|| format!("invalid {label}"))?;
    Ok(value.trim().to_string())
}
