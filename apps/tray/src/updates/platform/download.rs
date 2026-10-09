use anyhow::Result;
use std::io::Write;
use std::path::Path;
use tokio_stream::StreamExt;

use crate::daemon::{DaemonEvent, EventBus};

#[cfg(feature = "dev")]
pub(super) fn dev_update_url() -> Option<String> {
    std::env::var("QOL_TRAY_DEV_UPDATE_URL").ok()
}

#[cfg(not(feature = "dev"))]
pub(super) fn dev_update_url() -> Option<String> {
    None
}

pub(super) async fn download_asset(url: &str, dest: &Path, events: &EventBus) -> Result<()> {
    let request = crate::features::plugin_store::github::build_github_request(
        &crate::features::plugin_store::blob_client(),
        url,
        None,
    );
    let response = crate::features::plugin_store::github::send_checked(request).await?;
    let total = response.content_length();
    let mut stream = response.bytes_stream();
    let mut file = std::fs::File::create(dest)?;
    let mut downloaded: u64 = 0;
    let mut last_percent: u8 = 0;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        file.write_all(&chunk)?;
        downloaded += chunk.len() as u64;
        let percent = total
            .map(|t| ((downloaded * 100) / t).min(100) as u8)
            .unwrap_or(0);
        if percent != last_percent {
            events.send(DaemonEvent::UpdateProgress { percent });
            super::super::record_update_progress(percent);
            last_percent = percent;
        }
    }
    file.sync_all()?;
    Ok(())
}
