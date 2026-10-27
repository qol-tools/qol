use anyhow::Result;
use std::path::Path;
use std::time::Duration;

pub(crate) fn verify_candidate_runs(_binary: &Path, _timeout: Duration) -> Result<()> {
    Ok(())
}
