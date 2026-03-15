use anyhow::{anyhow, Result};

use super::super::model::AppRef;

pub(super) fn open_url_in_browser(_url: &str, _browser: &AppRef) -> Result<()> {
    Err(anyhow!(
        "browser override is not supported on this platform"
    ))
}

pub(super) fn launch_app(_app: &AppRef) -> Result<()> {
    Err(anyhow!("launch_app is not supported on this platform"))
}
