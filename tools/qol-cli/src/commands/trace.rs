use crate::workspace::repo_root;
use anyhow::{Context, Result};
use std::ffi::OsString;
use std::process::Command;

pub(crate) fn run(args: &[OsString]) -> Result<()> {
    let root = repo_root()?;
    let script_path = root.join("tools/compact_trace.py");

    if !script_path.is_file() {
        anyhow::bail!("trace script not found at {}", script_path.display());
    }

    let mut cmd = Command::new("python3");
    cmd.arg(&script_path);
    cmd.args(args);

    let status = cmd
        .current_dir(root)
        .status()
        .context("failed to run trace script")?;

    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }
    Ok(())
}
