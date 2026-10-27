use anyhow::{anyhow, Context, Result};
use qol_process::BoundedCommandOutput;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

const PROBE_OUTPUT_LIMIT: usize = 8192;
const UNRUNNABLE: &str =
    "This plugin build cannot run on this system; the installed version was kept";

pub(crate) fn verify_candidate_runs(binary: &Path, timeout: Duration) -> Result<()> {
    let mut command = Command::new(binary);
    command.arg("help");
    match qol_process::run_eager_candidate(command, timeout, PROBE_OUTPUT_LIMIT) {
        Ok(BoundedCommandOutput::Completed(_)) => Ok(()),
        Ok(BoundedCommandOutput::TimedOut { .. }) => Err(anyhow!(UNRUNNABLE)),
        Err(error) => Err(error).context(UNRUNNABLE),
    }
}
