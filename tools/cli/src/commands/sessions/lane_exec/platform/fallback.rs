use std::process::Command;

use anyhow::{bail, Result};

pub(crate) fn run_in_place(_command: Command) -> Result<()> {
    bail!("lane-exec is not supported on this platform")
}
