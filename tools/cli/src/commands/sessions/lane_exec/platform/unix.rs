use std::os::unix::process::CommandExt;
use std::process::Command;

use anyhow::Result;

/// Replaces this process with the lane, so the terminal's foreground process
/// is the harness itself.
pub(crate) fn run_in_place(mut command: Command) -> Result<()> {
    Err(command.exec().into())
}
