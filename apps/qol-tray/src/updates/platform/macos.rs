use anyhow::Result;
use std::sync::Arc;

use crate::daemon::EventBus;

use super::super::GITHUB_REPO;

pub(super) async fn download_and_install(_events: Arc<EventBus>) -> Result<()> {
    let url = format!("https://github.com/{}/releases/latest", GITHUB_REPO);
    crate::paths::open_url(&url)?;
    Ok(())
}
