use anyhow::{bail, Context, Result};
use qol_process::BoundedCommandOutput;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

const PROBE_OUTPUT_LIMIT: usize = 8192;
const UNRUNNABLE: &str = "This plugin build cannot run on this system";

pub(super) fn verify_candidate_runs(binary: &Path, timeout: Duration) -> Result<()> {
    let mut command = Command::new(binary);
    command.arg("help");
    qol_process::hide_console_window(&mut command);
    let output = qol_process::run_eager_candidate(command, timeout, PROBE_OUTPUT_LIMIT)
        .with_context(|| format!("{UNRUNNABLE} ({})", binary.display()))?;
    match output {
        BoundedCommandOutput::Completed(output) if output.status.success() => Ok(()),
        BoundedCommandOutput::Completed(output) => bail!(
            "{} ({}): {}",
            UNRUNNABLE,
            output.status,
            String::from_utf8_lossy(output.stderr.as_bytes()).trim()
        ),
        BoundedCommandOutput::TimedOut { .. } => {
            bail!("{UNRUNNABLE} (its help command did not finish)")
        }
    }
}
