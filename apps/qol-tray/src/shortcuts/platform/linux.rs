use anyhow::{anyhow, Result};

use super::super::model::AppRef;

pub(super) fn open_url_in_browser(url: &str, browser: &AppRef) -> Result<()> {
    let bin = match browser {
        AppRef::BundleId { id } => id.as_str(),
        AppRef::Path { path } => path.as_str(),
        AppRef::Name { name } => name.as_str(),
    };
    std::process::Command::new(bin)
        .arg(url)
        .spawn()
        .map_err(|e| anyhow!("failed to open url in browser: {}", e))?;
    Ok(())
}

pub(super) fn launch_app(app: &AppRef) -> Result<()> {
    let bin = match app {
        AppRef::BundleId { id } => id.as_str(),
        AppRef::Path { path } => path.as_str(),
        AppRef::Name { name } => name.as_str(),
    };
    std::process::Command::new(bin)
        .spawn()
        .map_err(|e| anyhow!("failed to launch app: {}", e))?;
    Ok(())
}
