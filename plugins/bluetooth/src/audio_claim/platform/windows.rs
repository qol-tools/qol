use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use anyhow::{anyhow, Result};

pub const RECLAIM_SUPPORTED: bool = false;

pub fn reclaim_output(_address: &str) -> Result<()> {
    Err(anyhow!(
        "plugin-bluetooth: audio reclaim is not implemented on Windows"
    ))
}

pub fn spawn_playback_watch(_enabled: Arc<AtomicBool>) {}
