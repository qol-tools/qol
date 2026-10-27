use anyhow::Result;
use std::path::Path;
use std::time::Duration;

mod platform;

pub(super) fn verify_candidate_runs(binary: &Path, timeout: Duration) -> Result<()> {
    platform::verify_candidate_runs(binary, timeout)
}
