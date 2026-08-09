use anyhow::{bail, Context, Result};
use std::ffi::OsString;
use std::path::Path;
use std::process::{Command, Stdio};

pub(crate) fn available() -> bool {
    Command::new("pkexec")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

pub(crate) fn run(label: &str, script: &str, args: &[String]) -> Result<()> {
    let mut command = Command::new("pkexec");
    command.args(["sh", "-c", script, label]);
    command.args(args);
    let status = command.status().context("failed to launch pkexec")?;
    if !status.success() {
        bail!("pkexec exited with {status}");
    }
    Ok(())
}

pub(crate) fn spawn(label: &str, program: &Path, args: &[OsString]) -> Result<std::process::Child> {
    let mut command = Command::new("pkexec");
    command.arg(program).args(args);
    command
        .spawn()
        .with_context(|| format!("failed to launch privileged {label}"))
}
