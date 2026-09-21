use anyhow::{anyhow, Result};

pub const RECLAIM_SUPPORTED: bool = false;

pub fn reclaim_output(_address: &str) -> Result<()> {
    Err(anyhow!(
        "{}: audio reclaim is not implemented on macOS",
        crate::PLUGIN_ID
    ))
}
