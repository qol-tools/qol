use anyhow::{bail, Context, Result};
use qol_process::{run_owned_with_output_timeout, BoundedCommandOutput};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

const PROBE_TIMEOUT: Duration = Duration::from_secs(10);
const PROBE_OUTPUT_LIMIT: usize = 8192;

pub(super) fn install(source: &Path, target: &Path, version: Option<&str>) -> Result<()> {
    let staged = stage(source, target)?;
    crate::updates::verify_host_update(
        &staged,
        version,
        qol_artifact::ArtifactExpectation::with_exact_target,
    )?;
    promote(&staged, target, version, PROBE_TIMEOUT)
}

fn stage(source: &Path, target: &Path) -> Result<tempfile::TempPath> {
    let parent = target
        .parent()
        .context("Update destination has no parent")?;
    let staged = tempfile::Builder::new()
        .prefix(".qol-tray-update-")
        .tempfile_in(parent)
        .context("Failed to create update staging file")?
        .into_temp_path();
    std::fs::copy(source, &staged).context("Failed to stage update")?;
    std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755))?;
    Ok(staged)
}

fn promote(staged: &Path, target: &Path, version: Option<&str>, timeout: Duration) -> Result<()> {
    if let Err(error) = verify_launch(staged, version, timeout) {
        log::error!("[self-update] compatibility probe rejected update: {error:#}");
        return Err(error.context(crate::updates::IncompatibleUpdate));
    }
    std::fs::rename(staged, target).context("Failed to replace installed tray")?;
    log::info!("[self-update] compatibility probe passed; installed verified update");
    Ok(())
}

fn verify_launch(binary: &Path, version: Option<&str>, timeout: Duration) -> Result<()> {
    let mut command = Command::new(binary);
    command.arg("--version").env("LD_BIND_NOW", "1");
    let output = run_owned_with_output_timeout(command, timeout, PROBE_OUTPUT_LIMIT)
        .context("Could not execute staged tray")?;
    let output = match output {
        BoundedCommandOutput::Completed(output) => output,
        BoundedCommandOutput::TimedOut { .. } => bail!("Staged tray version check timed out"),
    };
    if !output.status.success() {
        bail!(
            "Staged tray version check failed ({}): {}",
            output.status,
            String::from_utf8_lossy(output.stderr.as_bytes()).trim()
        );
    }
    let reply = std::str::from_utf8(output.stdout.as_bytes())?.trim();
    let actual = reply
        .strip_prefix("qol-tray ")
        .context("Unexpected staged tray version reply")?;
    semver::Version::parse(actual).context("Invalid staged tray version")?;
    if output.stdout.is_truncated() || version.is_some_and(|expected| expected != actual) {
        bail!("Staged tray version reply does not match the update");
    }
    Ok(())
}

#[cfg(test)]
mod tests;
