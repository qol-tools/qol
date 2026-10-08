use std::process::Command;

use anyhow::Result;

/// Windows cannot replace a process image, so the lane runs as a child and
/// its exit code becomes this process's exit code.
pub(crate) fn run_in_place(mut command: Command) -> Result<()> {
    let status = command.status()?;
    std::process::exit(status.code().unwrap_or(1));
}
