use anyhow::{bail, Context, Result};
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
